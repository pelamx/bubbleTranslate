//! Says when a newer build has been published. Never downloads or installs it.
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
