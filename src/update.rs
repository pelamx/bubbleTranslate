//! Says when a newer build has been published, and on Linux and Windows
//! installs it when asked: one click on "Update", never on its own.
//!
//! The downloads are assets on a versioned GitHub release, and `latest.json`
//! in the repository names both the version and the URL for each platform. A
//! release uploads the binary and rewrites its own platform's line, so "the
//! download on GitHub changed" and "a newer version is available" cannot
//! drift apart. The platforms are released separately, which is why each has
//! its own line: a rebuilt DMG tells macOS and nobody else.
//!
//! Windows points at the zip rather than the bare `.exe`, because this URL is
//! opened in the user's browser and a browser discards an unsigned `.exe` it
//! considers uncommon. The same bytes inside a zip arrive.
//!
//! The check reads a public file and sends nothing about this install.
//!
//! Installing downloads this platform's bare binary from the same release,
//! checks it against that release's `SHA256SUMS.txt`, puts it where the
//! running one is and starts it. The new copy then asks the old one to step
//! aside, the way [`crate::ipc`] already settles which of two copies runs, so
//! the restart is the ordinary startup and nothing here has to quit anything.
//! Only the program file changes: the licence, the settings and today's
//! allowance live elsewhere and stay as they are. macOS keeps the download
//! link, because the app there is a signed bundle inside a disk image.
//!
//! None of it runs in the Microsoft Store build, where the Store is what
//! publishes a new version and a packaged app may not rewrite its own files.
//! `main` does not call [`spawn`] there, so nothing ever fills [`AVAILABLE`]
//! and the rest of the module is unreachable rather than switched off one
//! function at a time. The module is still compiled, so it is still type-
//! checked and its tests still run; this says so rather than leaving a
//! column of dead-code warnings for the reader to work through.
#![cfg_attr(feature = "store", allow(dead_code))]

use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

const MANIFEST_URL: &str = "https://raw.githubusercontent.com/pelamx/downloads/main/latest.json";

/// A build newer than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Entry {
    version: String,
    url: String,
}

static AVAILABLE: Mutex<Option<Available>> = Mutex::new(None);

/// The newer build found by the last check, if there is one.
pub fn available() -> Option<Available> {
    AVAILABLE.lock().unwrap().clone()
}

/// Checks now and then once a day, off the calling thread. `found` runs when a
/// check turns something up, so the window can be redrawn to show it.
pub fn spawn(found: impl Fn() + Send + 'static) {
    #[cfg(debug_assertions)]
    if std::env::var_os("BUBBLETRANSLATE_UPDATE_URL").is_none() {
        // A development build is always "out of date" against the published
        // one, and the tests drive the real binary; neither needs the network.
        return;
    }
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            // The copy this one replaced may still be closing.
            std::thread::sleep(Duration::from_secs(10));
            tidy_after_update();
        }
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .user_agent(concat!("bubbleTranslate/", env!("CARGO_PKG_VERSION")))
                .build(),
        );
        loop {
            if let Some(newer) = check(&agent) {
                crate::trace!("update: {} is available", newer.version);
                *AVAILABLE.lock().unwrap() = Some(newer);
                found();
            }
            std::thread::sleep(Duration::from_secs(24 * 3600));
        }
    });
}

/// Where an install started from the window or the bubble has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Idle,
    Working,
    /// Could not be done here -- no network, a folder the app may not write
    /// to, a download that did not match. The download link is the way left.
    Failed,
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress::Idle);

pub fn progress() -> Progress {
    *PROGRESS.lock().unwrap()
}

/// Whether this platform can put the update in place itself.
///
/// Never in the Microsoft Store build: the files of a packaged app belong to
/// the package and may not be swapped underneath it, and the Store is what
/// delivers a new version there. `spawn` is not called in that build either,
/// so nothing reaches this — it is false as well because a constant that says
/// a Store copy can install its own update would be wrong wherever it is read.
pub const CAN_INSTALL: bool =
    cfg!(any(target_os = "linux", target_os = "windows")) && !cfg!(feature = "store");

/// Downloads and starts the newer build, off the calling thread. `changed`
/// runs when the outcome is known, so the window can be redrawn. On success
/// the new copy takes over and this one is asked to quit by it.
pub fn install(changed: impl Fn() + Send + 'static) {
    let Some(newer) = available() else { return };
    {
        let mut progress = PROGRESS.lock().unwrap();
        if *progress == Progress::Working {
            return;
        }
        *progress = Progress::Working;
    }
    std::thread::spawn(move || {
        let outcome = put_in_place(&newer);
        if let Err(err) = &outcome {
            crate::trace!("update: could not install {}: {err}", newer.version);
        }
        *PROGRESS.lock().unwrap() = match outcome {
            // Still working, as far as anyone looking can tell: the new copy
            // is starting and will close this one in a moment.
            Ok(()) => Progress::Working,
            Err(_) => Progress::Failed,
        };
        changed();
    });
}

/// The bare binary this platform installs, by its name in the release. On
/// Windows that is the `.exe` beside the zip the browser link points at: the
/// zip is only there for browsers, and this download is not one.
fn asset_name(os: &str) -> Option<&'static str> {
    match os {
        "linux" => Some("bubbleTranslate-linux-x86_64"),
        "windows" => Some("bubbleTranslate.exe"),
        _ => None,
    }
}

/// The checksum `SHA256SUMS.txt` gives for `name`, lowercase hex.
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64)
            .then(|| hash.to_ascii_lowercase())
    })
}

fn put_in_place(newer: &Available) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let name = asset_name(std::env::consts::OS).ok_or("not on this platform")?;
    // The release the manifest's link is in; `newer_in` already held it to
    // our own downloads.
    let (release, _) = newer.url.rsplit_once('/').ok_or("odd link")?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(concat!("bubbleTranslate/", env!("CARGO_PKG_VERSION")))
            .build(),
    );
    let sums = agent
        .get(format!("{release}/SHA256SUMS.txt"))
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let expected = checksum_for(&sums, name).ok_or("no checksum for it")?;
    let bytes = agent
        .get(format!("{release}/{name}"))
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != expected {
        return Err("download does not match its checksum".into());
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let fresh = exe.with_extension("new");
    std::fs::write(&fresh, &bytes).map_err(|e| format!("{}: {e}", fresh.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    swap(&fresh, &exe).inspect_err(|_| {
        let _ = std::fs::remove_file(&fresh);
    })?;

    // Started the way this one was, so a copy that runs in the background
    // goes on doing so. It finds this one running, sees it is newer, and
    // asks it to quit.
    std::process::Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|e| e.to_string())?;
    crate::trace!("update: {} is in place and starting", newer.version);
    Ok(())
}

/// Puts `fresh` where `exe` is. A running program's file can be replaced on
/// Linux but not on Windows, where it can only be moved aside: the old one is
/// renamed to `.old` and deleted by the next start.
fn swap(fresh: &std::path::Path, exe: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(fresh, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(e.to_string());
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    std::fs::rename(fresh, exe).map_err(|e| e.to_string())
}

/// The file an update on Windows moved aside. Gone by the time anyone looks,
/// once the copy that was running from it has quit.
#[cfg(target_os = "windows")]
fn tidy_after_update() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}

fn check(agent: &ureq::Agent) -> Option<Available> {
    let json = agent
        .get(manifest_url())
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    newer_in(&json, std::env::consts::OS, env!("CARGO_PKG_VERSION"))
}

fn manifest_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("BUBBLETRANSLATE_UPDATE_URL") {
        return url;
    }
    MANIFEST_URL.to_string()
}

/// Where every download lives. See CLAUDE.md for why it is this repository.
const DOWNLOADS: &str = "https://github.com/pelamx/downloads/releases/";

/// This platform's entry in the manifest, if it is newer than `running`. A
/// manifest that cannot be read, or has no line for this platform, is "nothing
/// new" rather than an error: the app works exactly as before without it.
fn newer_in(json: &str, os: &str, running: &str) -> Option<Available> {
    let manifest: std::collections::HashMap<String, Entry> = serde_json::from_str(json).ok()?;
    let entry = manifest.get(os)?;
    // Only ever a download from our own releases. The notice is a link the
    // user is invited to click, so a manifest edited by anyone who got hold of
    // a token must not be able to send every installed copy somewhere else.
    if !entry.url.starts_with(DOWNLOADS) {
        return None;
    }
    (parse(&entry.version)? > parse(running)?).then(|| Available {
        version: entry.version.clone(),
        url: entry.url.clone(),
    })
}

/// Whether `candidate` is a later release than `running`.
///
/// Shared with [`crate::ipc`], where a second copy of the binary and the one
/// already running have to work out which of them is the newer build. A
/// version either side cannot parse is not newer, so an unreadable answer
/// leaves whoever is running in place.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn is_newer(candidate: &str, running: &str) -> bool {
    match (parse(candidate), parse(running)) {
        (Some(candidate), Some(running)) => candidate > running,
        _ => false,
    }
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "linux":   { "version": "0.2.0",  "url": "https://github.com/pelamx/downloads/releases/download/v0.2.0/linux" },
        "macos":   { "version": "0.1.0",  "url": "https://github.com/pelamx/downloads/releases/download/v0.1.0/dmg" },
        "windows": { "version": "0.10.0", "url": "https://github.com/pelamx/downloads/releases/download/v0.10.0/exe" }
    }"#;

    #[test]
    fn a_newer_build_for_this_platform_is_reported() {
        let found = newer_in(MANIFEST, "linux", "0.1.0").unwrap();
        assert_eq!(found.version, "0.2.0");
        assert_eq!(
            found.url,
            "https://github.com/pelamx/downloads/releases/download/v0.2.0/linux"
        );
    }

    #[test]
    fn another_platforms_release_is_not_this_ones() {
        assert_eq!(newer_in(MANIFEST, "macos", "0.1.0"), None);
    }

    #[test]
    fn versions_compare_as_numbers_not_text() {
        assert!(newer_in(MANIFEST, "windows", "0.9.0").is_some());
        assert_eq!(newer_in(MANIFEST, "linux", "0.10.0"), None);
    }

    #[test]
    fn one_version_is_later_than_another_or_it_is_not() {
        assert!(is_newer("0.2.3", "0.2.2"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.2.2", "0.2.2"));
        assert!(!is_newer("0.2.1", "0.2.2"));
        // Nothing to compare is nothing to act on.
        assert!(!is_newer("", "0.2.2"));
        assert!(!is_newer("soon", "0.2.2"));
        assert!(!is_newer("0.2.3", "nightly"));
    }

    #[test]
    fn the_checksum_is_found_by_file_name() {
        let sums = "d3a8 *bubbleTranslate.dmg\n\
            8c2c54dffae2c48f57bdd063e25744ccc29fb5b4ca3c1609df25b817aaefb735 *bubbleTranslate.exe\n\
            DCB9BF9866869FB9F350BBD4C91D6FBF0057EA3F0CB0AC651C71EB913B9D2074  bubbleTranslate-linux-x86_64\n";
        assert_eq!(
            checksum_for(sums, "bubbleTranslate-linux-x86_64").as_deref(),
            Some("dcb9bf9866869fb9f350bbd4c91d6fbf0057ea3f0cb0ac651c71eb913b9d2074")
        );
        assert!(checksum_for(sums, "bubbleTranslate.exe").is_some());
        // A line too short to be a SHA-256 is not one.
        assert_eq!(checksum_for(sums, "bubbleTranslate.dmg"), None);
        assert_eq!(checksum_for(sums, "bubbleTranslate-windows-x64.zip"), None);
    }

    #[test]
    fn a_broken_manifest_is_nothing_new() {
        assert_eq!(newer_in("<html>", "linux", "0.1.0"), None);
        assert_eq!(
            newer_in(
                r#"{"linux":{"version":"soon","url":"https://x"}}"#,
                "linux",
                "0.1.0"
            ),
            None
        );
        assert_eq!(
            newer_in(
                r#"{"linux":{"version":"9.0.0","url":"http://x"}}"#,
                "linux",
                "0.1.0"
            ),
            None
        );
    }

    #[test]
    fn a_link_anywhere_but_our_releases_is_ignored() {
        for url in [
            "https://evil.example/bubbleTranslate",
            "https://github.com/someone/downloads/releases/download/v9.0.0/x",
            "https://github.com/pelamx/downloads.evil.example/releases/x",
        ] {
            let json = format!(r#"{{"linux":{{"version":"9.0.0","url":"{url}"}}}}"#);
            assert_eq!(newer_in(&json, "linux", "0.1.0"), None, "{url}");
        }
    }
}
