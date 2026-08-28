//! Telling the window manager what the bubble is.
//!
//! By default a window manager treats every window as an ordinary one: it
//! gives it a border, puts it in the alt-tab list, and focuses it when it
//! appears. All three are wrong for the bubble. The worst is the focus — the
//! application being read loses it, and with it the selection the translation
//! was about.
//!
//! EWMH has a hint for exactly this shape of window, and notification daemons
//! have used it for twenty years, so support is close to universal. The window
//! is declared a notification, asked to stay above others, to stay out of the
//! taskbar, and to appear on every workspace — text gets selected wherever the
//! user is working, and a bubble that only shows up on the workspace the app
//! was started on is no use — and marked as never wanting keyboard input.
//!
//! The last of those is the one hint that is not enough on its own. wlroots
//! compositors ignore `_NET_WM_STATE_STICKY` for XWayland clients, and hold
//! the equivalent state themselves rather than exposing it as a property, so
//! it has to be asked for over their IPC instead — see
//! [`keep_on_all_workspaces`].
//!
//! Timing matters: a window manager reads these when the window is mapped, and
//! changing them afterwards may be ignored. The bubble is created hidden and
//! only mapped when there is something to say, which leaves a comfortable
//! window in which to get this done.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// `WM_HINTS.flags`: the input field below is meaningful.
const INPUT_HINT: u32 = 1;

/// `_NET_WM_DESKTOP`: show on all of them.
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

/// Puts the bubble on every workspace, by whatever means this compositor has.
///
/// `_NET_WM_STATE_STICKY` is set in [`mark_as_notification`] and most window
/// managers honour it — the X11 ones essentially all do. wlroots-based
/// compositors do not implement it for XWayland clients at all. Their
/// equivalent is a *pin*, which is not a property a client can set on itself;
/// it is a state the compositor holds, reachable only through that
/// compositor's own IPC. So where such an IPC exists, this asks.
///
/// That is worth doing rather than documenting a config line, because the
/// symptom is quiet and confusing: the translator keeps working, the trace
/// keeps saying it produced a bubble, and nothing appears — because the bubble
/// is sitting on the workspace it was first mapped on.
///
/// Returns false while the answer is "not yet", so the caller retries; true
/// once the bubble is pinned or once there is nothing left to try.
pub fn keep_on_all_workspaces() -> bool {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        // The sticky hint went out with the other properties and there is no
        // second channel to try.
        return true;
    }
    hyprland_pin()
}

/// The bubble's window title, which is the app name `run_native` is given.
/// The main window sets its own (`Bubble Translate`), so the two are told
/// apart by it — the compositor reports no X11 window id to match on instead.
const BUBBLE_TITLE: &str = "bubbleTranslate";

/// Asks Hyprland to pin the bubble.
///
/// Rate-limited, because the caller retries on every frame the bubble is
/// visible and each attempt is a process. The failure count is what stops a
/// session that will never pin — a compositor that renamed its JSON, a title
/// that no longer matches — from shelling out for the rest of its life; it
/// resets on success, so the re-assert after each hide costs two calls and not
/// a fresh budget of forty.
fn hyprland_pin() -> bool {
    use std::time::{Duration, Instant};

    const RETRY_INTERVAL: Duration = Duration::from_millis(250);
    const MAX_FAILURES: u32 = 40;

    #[derive(Default)]
    struct PinState {
        last: Option<Instant>,
        failures: u32,
        /// Whether this has ever worked. Gates the trace, so a bubble shown
        /// fifty times does not report the same success fifty times.
        succeeded: bool,
        gave_up: bool,
    }

    static STATE: std::sync::Mutex<Option<PinState>> = std::sync::Mutex::new(None);

    let Ok(mut guard) = STATE.lock() else {
        return true;
    };
    let state = guard.get_or_insert_with(PinState::default);

    if state.gave_up {
        return true;
    }
    if let Some(last) = state.last {
        if last.elapsed() < RETRY_INTERVAL {
            return false;
        }
    }
    state.last = Some(Instant::now());

    match pin_via_hyprctl() {
        Ok(true) => {
            if !state.succeeded {
                crate::trace!("window: bubble pinned to every workspace");
                state.succeeded = true;
            }
            state.failures = 0;
            true
        }
        Ok(false) => {
            state.failures += 1;
            if state.failures >= MAX_FAILURES {
                crate::trace!(
                    "window: gave up pinning the bubble; \
                     a `windowrule = pin, class:^(bubbleTranslate)$` line does the same"
                );
                state.gave_up = true;
                return true;
            }
            false
        }
        Err(err) => {
            crate::trace!("window: could not pin the bubble: {err}");
            state.failures += 1;
            false
        }
    }
}

fn pin_via_hyprctl() -> Result<bool, String> {
    let out = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("hyprctl clients failed".into());
    }
    let clients: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;

    let pid = std::process::id() as i64;
    let bubble = clients.iter().find(|c| {
        c.get("pid").and_then(|v| v.as_i64()) == Some(pid)
            && c.get("title").and_then(|v| v.as_str()) == Some(BUBBLE_TITLE)
    });

    // Not mapped yet — the bubble only exists while it has something to say.
    // The caller keeps asking.
    let Some(bubble) = bubble else {
        return Ok(false);
    };

    if bubble.get("pinned").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(true);
    }

    let address = bubble
        .get("address")
        .and_then(|v| v.as_str())
        .ok_or("client has no address")?;
    let target = format!("address:{address}");

    // Pinning only applies to floating windows — a tiled one has a place in
    // the layout of one workspace by definition. The bubble is floating
    // wherever the notification hint is honoured, so this is for the
    // compositor that tiled it anyway, and it is best effort: if it does not
    // take, the pin below says so.
    if bubble.get("floating").and_then(|v| v.as_bool()) != Some(true) {
        let _ = dispatch(&["setfloating", &target]);
    }

    // Hyprland speaks two dialects of this. Up to 0.55 a dispatch is a string
    // — `pin address:0x…` — and from 0.56 the same request is a Lua call
    // against a typed window handle, with the old spelling rejected as a
    // syntax error. Which one this build wants is not worth detecting: try the
    // older, and if it is refused, say it the newer way.
    if dispatch(&["pin", &target]).is_err() {
        eval_pin(pid)?;
    }

    // Not `true`: the request has been made, not confirmed. The next pass
    // re-reads `pinned` above and settles it, which is the difference between
    // reporting that the bubble will follow the user and reporting that we
    // asked for it to.
    Ok(false)
}

fn dispatch(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("hyprctl")
        .arg("dispatch")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("hyprctl dispatch {} failed", args.join(" ")));
    }
    Ok(())
}

/// The 0.56-and-later spelling: find our own window among the compositor's and
/// hand the handle to the pin dispatcher.
///
/// Matched on pid and title here as well, rather than on the address already
/// in hand, because the Lua API offers no lookup by address — and the pid is
/// the stronger half of the match in any case.
fn eval_pin(pid: i64) -> Result<(), String> {
    let script = format!(
        "for _, w in ipairs(hl.get_windows()) do \
         if w.pid == {pid} and w.title == '{BUBBLE_TITLE}' then \
         hl.dispatch(hl.dsp.window.pin(w)) end end"
    );
    let out = std::process::Command::new("hyprctl")
        .arg("eval")
        .arg(&script)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("hyprctl eval failed".into());
    }
    Ok(())
}

/// Declares `window` a notification: no decorations, no taskbar entry, always
/// above, and never focused.
///
/// Best effort by nature — a window manager is free to ignore any of it — so
/// nothing here is worth failing over. What it costs when it does not work is
/// a border and a focus steal, not a broken translator.
pub fn mark_as_notification(window: u32) {
    if let Err(err) = mark(window) {
        crate::trace!("window: could not mark the bubble as a notification: {err}");
    }
}

fn mark(window: u32) -> Result<(), String> {
    let (conn, _) = x11rb::connect(None).map_err(|e| e.to_string())?;

    let atom = |name: &str| -> Result<u32, String> {
        conn.intern_atom(false, name.as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map(|r| r.atom)
            .map_err(|e| e.to_string())
    };

    set(
        &conn,
        window,
        atom("_NET_WM_WINDOW_TYPE")?,
        &[atom("_NET_WM_WINDOW_TYPE_NOTIFICATION")?],
    )?;

    set(
        &conn,
        window,
        atom("_NET_WM_STATE")?,
        &[
            atom("_NET_WM_STATE_ABOVE")?,
            atom("_NET_WM_STATE_STICKY")?,
            atom("_NET_WM_STATE_SKIP_TASKBAR")?,
            atom("_NET_WM_STATE_SKIP_PAGER")?,
        ],
    )?;

    conn.change_property32(
        PropMode::REPLACE,
        window,
        atom("_NET_WM_DESKTOP")?,
        u32::from(AtomEnum::CARDINAL),
        &[ALL_DESKTOPS],
    )
    .map_err(|e| e.to_string())?;

    // The input hint is the older half of the "do not focus me" convention and
    // the half that predates EWMH, so some window managers honour only this.
    // The nine words are the whole `WM_HINTS` structure; every field after the
    // first two is unset, which the flags say to ignore.
    conn.change_property32(
        PropMode::REPLACE,
        window,
        u32::from(AtomEnum::WM_HINTS),
        u32::from(AtomEnum::WM_HINTS),
        &[INPUT_HINT, 0, 0, 0, 0, 0, 0, 0, 0],
    )
    .map_err(|e| e.to_string())?;

    conn.flush().map_err(|e| e.to_string())?;
    crate::trace!("window: bubble marked as a notification");
    Ok(())
}

fn set(conn: &RustConnection, window: u32, property: u32, atoms: &[u32]) -> Result<(), String> {
    conn.change_property32(
        PropMode::REPLACE,
        window,
        property,
        u32::from(AtomEnum::ATOM),
        atoms,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
