//! Desktop shell integration: autostart, opening links, and the stubs the UI
//! shares with macOS and Windows.
//!
//! Linux has no tray icon. The app is the window: closing it quits, and
//! running the binary again (or the launcher entry) brings it back.

/// Linux has nothing outside the app's own windows to bring it back, so
/// closing the main window quits.
pub fn has_indicator() -> bool {
    false
}

/// Marks the autostart entry as ours, so switching it off never deletes a
/// file the user wrote by hand under the same name.
const AUTOSTART_MARK: &str = "X-bubbleTranslate-Autostart=true";

/// Writes or removes `~/.config/autostart/bubbleTranslate.desktop`, the
/// freedesktop way to start at login, which GNOME, KDE, XFCE and most
/// compositors' session tools honour.
///
/// `--background` is passed for parity with the other systems; with no tray
/// the app opens the window anyway, so this can never start something
/// unreachable.
pub fn set_start_at_login(on: bool) {
    let Some(dir) = dirs::config_dir().map(|d| d.join("autostart")) else {
        return;
    };
    let path = dir.join("bubbleTranslate.desktop");
    let ours = std::fs::read_to_string(&path).is_ok_and(|text| text.contains(AUTOSTART_MARK));
    if !on {
        if ours {
            let _ = std::fs::remove_file(&path);
        }
        return;
    }
    if path.exists() && !ours {
        // Someone wrote their own; theirs wins.
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=bubbleTranslate\n\
         Comment=Select text anywhere and a bubble appears with the translation\n\
         Exec=\"{}\" --background\n\
         Icon=bubbleTranslate\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         {AUTOSTART_MARK}\n",
        exe.display()
    );
    if std::fs::read_to_string(&path).is_ok_and(|text| text == entry) {
        return;
    }
    if let Err(err) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, entry)) {
        crate::trace!("autostart: could not write {}: {err}", path.display());
    }
}

/// Opens a URL in whatever the user's browser is, and brings it forward.
///
/// The one outward link the app has. Buying happens in a browser and nowhere
/// else: 3-D Secure needs one, and an app that never asks for a card number is
/// an app with nothing to leak.
pub fn open_url(url: &str) {
    let child = match std::process::Command::new("xdg-open").arg(url).spawn() {
        Ok(child) => child,
        Err(err) => {
            eprintln!("bubbleTranslate: could not open {url}: {err}");
            return;
        }
    };

    // What is left to do blocks -- waiting on the child, then asking the
    // compositor to raise the browser -- and neither is worth a dropped frame
    // on the bubble.
    std::thread::spawn(move || {
        let mut child = child;
        // `xdg-open` exits as soon as it has handed the URL over, and nothing
        // was reaping it: every click left a defunct process parented to us
        // for the rest of the session.
        let _ = child.wait();
        raise_browser();
    });
}

/// Whether the browser window has been asked to come forward yet.
enum Focus {
    /// Asked. Nothing further to do.
    Done,
    /// The browser has no window yet -- it is still starting. Ask again.
    NotYet,
    /// Not a session this can drive. The page still opened.
    Unsupported,
}

/// Brings the browser forward once the URL has been handed to it.
///
/// A URL handed to a browser that is already running becomes a tab in whatever
/// workspace that browser already occupies, and it arrives with no
/// xdg-activation token -- so the compositor has no reason to raise it. On a
/// session with more than one workspace the click then reads as having done
/// nothing at all, which is the one thing a buy button must not do.
///
/// Best effort from end to end: where this cannot be driven the page has still
/// opened, and that is not a failure worth printing at someone.
fn raise_browser() {
    let Some(class) = browser_window_class() else {
        return;
    };
    // The window is there at once when the browser was already running, and a
    // second or two late when it had to be started for this.
    for _ in 0..15 {
        match focus_by_class(&class) {
            Focus::Done | Focus::Unsupported => return,
            Focus::NotYet => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
}

/// `hyprctl <args>`, run under [`super::IPC_BUDGET`] and read as a refusal
/// when it does not answer. This runs while a browser is being raised on the
/// engine thread; a wedged compositor must end the attempt, not park it.
fn hyprctl(args: &[&str]) -> Option<std::process::Output> {
    let mut command = std::process::Command::new("hyprctl");
    command.args(args);
    super::timed_output(command, super::IPC_BUDGET).ok()
}

fn focus_by_class(class: &str) -> Focus {
    // A class is interpolated into a Lua string below, so refuse the
    // characters that would end it early rather than build a broken script.
    if class.contains('\'') || class.contains('\\') {
        return Focus::Unsupported;
    }

    let out = match hyprctl(&["clients", "-j"]) {
        Some(out) => out,
        None => return Focus::Unsupported,
    };
    if !out.status.success() {
        return Focus::Unsupported;
    }
    let Ok(clients) = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout) else {
        return Focus::Unsupported;
    };
    if !clients
        .iter()
        .any(|c| c.get("class").and_then(|v| v.as_str()) == Some(class))
    {
        return Focus::NotYet;
    }

    // The same two dialects the bubble's pin has to straddle: up to Hyprland
    // 0.55 a dispatch is a string, and from 0.56 it is a Lua call against a
    // window handle, with the older spelling rejected as a syntax error rather
    // than quietly ignored -- so a failure here is safe to fall through on.
    let target = format!("class:{class}");
    let old = hyprctl(&["dispatch", "focuswindow", &target]);
    if matches!(&old, Some(out) if out.status.success()) {
        return Focus::Done;
    }

    // `done` rather than a bare loop body: a browser with several windows
    // open would otherwise be focused once per window, ending on whichever
    // the compositor happened to enumerate last.
    let script = format!(
        "local done = false \
         for _, w in ipairs(hl.get_windows()) do \
         if not done and w.class == '{class}' then \
         hl.dispatch(hl.dsp.focus({{ window = w }})) done = true end end"
    );
    let _ = hyprctl(&["eval", &script]);
    Focus::Done
}

/// The window class of the user's default browser, from the desktop entry
/// `xdg-settings` names.
///
/// `StartupWMClass` is the entry's own statement of what its window will be
/// called, which is exactly the question being asked; the executable name in
/// `Exec` is the fallback, and is what most browsers use anyway.
fn browser_window_class() -> Option<String> {
    // `BROWSER` is cleared for the query: when it is set, `xdg-settings`
    // reports it back rather than the desktop entry, and what is wanted here
    // is the entry -- it is the only thing that names a window class.
    let out = std::process::Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .env_remove("BROWSER")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let entry = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if entry.is_empty() || entry.contains('/') {
        return None;
    }

    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/.local/share/applications"));
    }
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| format!("{d}/applications")),
    );

    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(format!("{dir}/{entry}")) else {
            continue;
        };
        if let Some(class) = desktop_value(&text, "StartupWMClass") {
            return Some(class);
        }
        return desktop_value(&text, "Exec").and_then(|exec| {
            exec.split_whitespace()
                .next()
                .and_then(|bin| bin.rsplit('/').next())
                .filter(|bin| !bin.is_empty())
                .map(str::to_string)
        });
    }
    None
}

/// The first value for `key` in a desktop entry. First, because the entry
/// repeats `Exec` once per action and the one that runs the program is the one
/// at the top.
fn desktop_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Always settled: there is no tray to wait for.
pub fn indicator_settled() -> bool {
    true
}

/// There is no tray menu to ask.
pub fn take_open_request() -> bool {
    false
}

/// There is no tray menu to ask.
pub fn take_quit_request() -> bool {
    false
}

/// Nothing to install; kept so the UI calls the same sequence everywhere.
pub fn install(_ctx: eframe::egui::Context) {}

/// No-ops, so the UI can call the same sequence it does on macOS.
///
/// Their macOS counterparts move the app between background and foreground
/// activation policies, a distinction X11 window managers do not have — a
/// window is raised by asking for it, which the UI does with a viewport
/// command right after calling these.
pub fn set_foreground(_visible: bool) {}

pub fn activate() {}

pub fn run_in_background() {}

/// Nothing to hook, so the UI's retry loop stops on the first frame. The macOS
/// counterpart intercepts the Finder's "reopen" event.
pub fn hook_reopen() -> bool {
    true
}

#[cfg(test)]
mod tests {
    //! The session-reading test is ignored by default: it only means
    //! anything on a live desktop. Run it with
    //! `cargo test -- --ignored --nocapture`.

    #[test]
    fn desktop_value_reads_the_first_match_only() {
        let entry = "[Desktop Entry]\nExec=brave-origin %U\nStartupWMClass=brave-origin\n\n[Desktop Action new]\nExec=brave-origin --incognito\n";
        assert_eq!(
            super::desktop_value(entry, "StartupWMClass").as_deref(),
            Some("brave-origin")
        );
        // The action's Exec must not win over the entry's own.
        assert_eq!(
            super::desktop_value(entry, "Exec").as_deref(),
            Some("brave-origin %U")
        );
        assert_eq!(super::desktop_value(entry, "NoSuchKey"), None);
    }

    #[test]
    #[ignore]
    fn resolves_this_session_browser_class() {
        let class = super::browser_window_class();
        println!("browser_window_class() = {class:?}");
        assert!(class.is_some(), "no browser class resolved on this session");
    }
}
