//! Putting the download where it belongs.
//!
//! The Linux release is one executable, and a browser saves it in Downloads,
//! which is not where anything looks for an application. So the first time it
//! is started from anywhere else it copies itself to `~/.local/bin`, writes a
//! launcher and an icon where the application menu finds them, and carries on
//! from the installed copy. Running a newer download over an older install is
//! then the whole of updating by hand: the copy is replaced, and the running
//! older one is asked to stand down as it always is.
//!
//! Everything lands in the user's own XDG directories, as `linux/install.sh`
//! does, so nothing needs root.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// Marks the launcher as ours, so a hand-written one under the same name is
/// never overwritten.
const LAUNCHER_MARK: &str = "X-bubbleTranslate-Launcher=true";

const DESKTOP_TEMPLATE: &str = include_str!("../../../linux/bubbleTranslate.desktop");
const ICON: &str = include_str!("../../../linux/bubbleTranslate.svg");

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn bin_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_BIN_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/bin")))
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/share")))
}

/// Where the installed copy lives.
pub fn installed_path() -> Option<PathBuf> {
    bin_dir().map(|d| d.join("bubbleTranslate"))
}

/// Installs this copy if it is not the installed one and then becomes it;
/// returns only when it is already installed or installing is not wanted.
///
/// Not for a development build, which runs from `target/` on purpose, nor for
/// a copy a package manager put under `/usr`, `/opt` or `/nix`, which is
/// managed by something else.
pub fn install_and_relaunch(args: &[String]) {
    if cfg!(debug_assertions) || std::env::var_os("BUBBLETRANSLATE_NO_INSTALL").is_some() {
        return;
    }
    let (Ok(exe), Some(target)) = (std::env::current_exe(), installed_path()) else {
        return;
    };
    let exe = exe.canonicalize().unwrap_or(exe);
    if ["/usr/", "/opt/", "/nix/"]
        .iter()
        .any(|p| exe.starts_with(p))
    {
        return;
    }
    if target.canonicalize().is_ok_and(|t| t == exe) {
        write_launcher(&target);
        return;
    }

    // An older download started by mistake must not take the place of a
    // newer install; the installed copy is the one to run.
    let keep_installed = installed_version(&target)
        .is_some_and(|v| crate::update::is_newer(&v, env!("CARGO_PKG_VERSION")));
    if !keep_installed {
        if let Err(err) = copy_into_place(&exe, &target) {
            crate::trace!("install: could not install to {}: {err}", target.display());
            return;
        }
        crate::trace!("install: installed to {}", target.display());
    }
    write_launcher(&target);

    let err = std::process::Command::new(&target).args(args).exec();
    // Only reached when the installed copy could not be started; this one
    // runs instead rather than nothing running.
    crate::trace!("install: could not start {}: {err}", target.display());
}

fn installed_version(target: &Path) -> Option<String> {
    let out = std::process::Command::new(target)
        .arg("--version")
        .env("BUBBLETRANSLATE_NO_INSTALL", "1")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().last().map(str::to_string)
}

/// Copied beside the target and renamed over it, so the swap is atomic and a
/// copy of the old version that is still running keeps its own file.
fn copy_into_place(exe: &Path, target: &Path) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let fresh = dir.join(".bubbleTranslate.new");
    std::fs::copy(exe, &fresh)?;
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&fresh, target)
}

/// The launcher names the installed copy by its full path, because
/// `~/.local/bin` is on the PATH of a terminal far more often than on the
/// PATH a desktop launches applications with.
fn launcher(target: &Path) -> String {
    let mut text: String = DESKTOP_TEMPLATE
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec=\"{}\"\n", target.display())
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    text.push_str(LAUNCHER_MARK);
    text.push('\n');
    text
}

fn write_launcher(target: &Path) {
    let Some(data) = data_dir() else {
        return;
    };
    let apps = data.join("applications");
    let path = apps.join("bubbleTranslate.desktop");
    let entry = launcher(target);
    match std::fs::read_to_string(&path) {
        Ok(text) if text == entry => {}
        // Someone wrote their own; theirs wins.
        Ok(text) if !text.contains(LAUNCHER_MARK) => {}
        _ => {
            if let Err(err) =
                std::fs::create_dir_all(&apps).and_then(|_| std::fs::write(&path, entry))
            {
                crate::trace!("install: could not write {}: {err}", path.display());
            }
            // Only some desktops need this, and it is harmless elsewhere.
            let _ = std::process::Command::new("update-desktop-database")
                .arg(&apps)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    let icon = data.join("icons/hicolor/scalable/apps/bubbleTranslate.svg");
    if std::fs::read_to_string(&icon).is_ok_and(|text| text == ICON) {
        return;
    }
    if let Some(dir) = icon.parent()
        && let Err(err) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&icon, ICON))
    {
        crate::trace!("install: could not write {}: {err}", icon.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_names_the_installed_copy_by_its_full_path() {
        let text = launcher(Path::new("/home/someone/.local/bin/bubbleTranslate"));
        assert!(text.contains("Exec=\"/home/someone/.local/bin/bubbleTranslate\"\n"));
        assert_eq!(text.matches("Exec=").count(), 1);
        assert!(text.contains("Name=bubbleTranslate"));
        assert!(text.ends_with(&format!("{LAUNCHER_MARK}\n")));
    }
}
