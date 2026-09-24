//! Asking the compositor what the keyboard is doing.
//!
//! No Wayland protocol will answer this. That is deliberate — a client that
//! could read the keyboard without focus is a keylogger — and it is why the
//! trigger key otherwise needs `/dev/input`, which needs a group membership
//! the desktop does not hand out.
//!
//! But a compositor with its own IPC is not bound by what the protocols
//! standardise, and Hyprland's exposes exactly the question worth asking:
//! `hl.is_key_down`. So on Hyprland the gate costs one round trip over a
//! socket the session already owns — no permission, no configuration, and
//! nothing left behind in the compositor.
//!
//! The same shape as [`super::cursor`], and for the same reason: a compositor
//! that grows a way to answer needs one arm here, not a dependency.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::TriggerKey;

/// How long to wait on the compositor. Generous for a local socket and short
/// enough that a wedged compositor cannot hold up a selection: an unanswered
/// question becomes "no idea", and the gate opens rather than jamming shut.
const TIMEOUT: Duration = Duration::from_millis(150);

/// The keysyms that count as this key, either side of the keyboard.
fn keysyms(key: TriggerKey) -> Option<[&'static str; 2]> {
    Some(match key {
        TriggerKey::Always => return None,
        TriggerKey::Shift => ["Shift_L", "Shift_R"],
        TriggerKey::Ctrl => ["Control_L", "Control_R"],
        TriggerKey::Alt => ["Alt_L", "Alt_R"],
        TriggerKey::Super => ["Super_L", "Super_R"],
    })
}

/// Whether the trigger key is held down right now, as the compositor sees it.
///
/// `None` when there is no compositor to ask or it would not answer, which the
/// caller reads as "no evidence" rather than "not held".
pub fn key_held(key: TriggerKey) -> Option<bool> {
    let [left, right] = keysyms(key)?;
    // `eval` answers every successful evaluation with a bare "ok" and throws
    // the value away, so the answer is raised as an error instead — the one
    // channel that carries a string back out. Both sides are deliberate:
    // Hyprland treats this as a failed evaluation, and nothing in the
    // compositor is changed either way.
    let lua = format!(
        "if hl.is_key_down('{left}') or hl.is_key_down('{right}') \
         then error('BT_HELD') else error('BT_FREE') end"
    );
    let reply = hyprland_eval(&lua)?;
    if reply.contains("BT_HELD") {
        Some(true)
    } else if reply.contains("BT_FREE") {
        Some(false)
    } else {
        // An older Hyprland without the Lua API, or one that answered
        // something else entirely. Not knowing is the honest reading.
        None
    }
}

/// Sends Ctrl+C to whatever has keyboard focus, as the compositor itself.
///
/// Not a virtual keyboard: on one of those the modifiers the user is still
/// physically holding merge into the chord at the seat, and Shift held from
/// the selection would turn this into Ctrl+Shift+C — which opens the
/// inspector in a browser. `send_key_state` carries its own modifier mask.
/// The press and release are split for the reason Omarchy's own copy binding
/// splits them: `send_shortcut` can leave the key stuck repeating.
pub fn send_copy() -> bool {
    let lua = "hl.dispatch(hl.dsp.send_key_state({ mods = 'CTRL', key = 'C', state = 'down' })) \
               hl.timer(function() \
                 hl.dispatch(hl.dsp.send_key_state({ mods = 'CTRL', key = 'C', state = 'up' })) \
               end, { timeout = 30, type = 'oneshot' }) \
               error('BT_SENT')";
    hyprland_eval(lua).is_some_and(|reply| reply.contains("BT_SENT"))
}

/// The class of the window with keyboard focus, lowercased.
///
/// `None` when there is no focused window or the compositor will not say.
pub fn focused_class() -> Option<String> {
    let lua = "local w = hl.get_active_window() \
               if not w then error('BT_NONE') end \
               error('BT_CLASS=' .. tostring(w.class) .. '=BT_END')";
    let reply = hyprland_eval(lua)?;
    let (_, rest) = reply.split_once("BT_CLASS=")?;
    let (class, _) = rest.split_once("=BT_END")?;
    Some(class.to_lowercase())
}

/// Whether this session has a compositor that can be asked at all.
pub fn can_answer() -> bool {
    socket_path().is_some_and(|path| path.exists())
}

/// Hyprland's control socket for this instance.
fn socket_path() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let mut path = PathBuf::from(runtime);
    path.push("hypr");
    path.push(signature);
    path.push(".socket.sock");
    Some(path)
}

/// Runs one line of Lua in the compositor and returns what it said.
fn hyprland_eval(lua: &str) -> Option<String> {
    let stream = UnixStream::connect(socket_path()?).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;

    let mut stream = stream;
    stream.write_all(format!("eval {lua}").as_bytes()).ok()?;
    stream.flush().ok()?;

    let mut reply = String::new();
    // Bounded: the reply is one short line, and a compositor that decided to
    // talk forever must not be able to grow this thread.
    Read::by_ref(&mut stream)
        .take(4096)
        .read_to_string(&mut reply)
        .ok()?;
    Some(reply)
}

/// How the key that reads the screen reaches us on Hyprland.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScreenBind {
    /// Our binding was already there: Hyprland ran it, and swallowed the key.
    Live,
    /// It was missing — a config reload clears it — and has just been put
    /// back. This press was not swallowed and not delivered, so whoever saw it
    /// has to pass it on themselves.
    Added,
    /// Not Hyprland, or the combination is the user's own, or the compositor
    /// would not take the binding. The key is read from `/dev/input` instead.
    None,
}

/// What our binding is called, which is how it is found again: a binding made
/// through the Lua API is listed by its description, not by what it runs.
const READ_SCREEN_DESCRIPTION: &str = "bubbleTranslate: read the screen";

/// Makes sure Ctrl+Shift+E is bound, in the compositor, to
/// `bubbleTranslate --read-screen`.
///
/// Worth it for two reasons. A binding is consumed by the compositor, so the
/// window underneath never sees the key — reading it from `/dev/input` cannot
/// stop VS Code opening its explorer at the same moment. And it needs no
/// group membership at all.
///
/// A combination the user has bound to something of their own is left alone.
pub fn ensure_read_screen_bind() -> ReadScreenBind {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return ReadScreenBind::None;
    }
    let Some(binds) = hyprland_binds() else {
        return ReadScreenBind::None;
    };
    match classify(&binds) {
        Chord::Ours => return ReadScreenBind::Live,
        Chord::Theirs => return ReadScreenBind::None,
        Chord::Free => {}
    }

    let Some(command) = read_screen_command() else {
        return ReadScreenBind::None;
    };
    // 0.56 and later configure in Lua; before that, in bind strings. Long
    // brackets, so nothing in the path needs escaping for Lua.
    let lua = format!(
        "hl.bind('CTRL + SHIFT + E', hl.dsp.exec_cmd([==[{command}]==]), \
         {{ description = '{READ_SCREEN_DESCRIPTION}' }})"
    );
    if !hyprctl_ok(&["eval", &lua]) {
        let _ = hyprctl_ok(&[
            "keyword",
            "bind",
            &format!("CTRL SHIFT, E, exec, {command}"),
        ]);
    }
    match hyprland_binds().map(|binds| classify(&binds)) {
        Some(Chord::Ours) => {
            crate::trace!("hotkey    ctrl+shift+e bound in hyprland");
            ReadScreenBind::Added
        }
        _ => ReadScreenBind::None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Chord {
    Ours,
    Theirs,
    Free,
}

/// Who, if anyone, has Ctrl+Shift+E in this list of bindings.
fn classify(binds: &[serde_json::Value]) -> Chord {
    /// Hyprland's modifier mask: Shift is 1, Control is 4.
    const CTRL_SHIFT: i64 = 1 | 4;
    let text = |bind: &serde_json::Value, key: &str| {
        bind.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let mut theirs = false;
    for bind in binds {
        let ours = text(bind, "description") == READ_SCREEN_DESCRIPTION
            || (text(bind, "dispatcher") == "exec" && text(bind, "arg").ends_with("--read-screen"));
        if ours {
            return Chord::Ours;
        }
        if text(bind, "key").eq_ignore_ascii_case("e")
            && bind.get("modmask").and_then(|v| v.as_i64()) == Some(CTRL_SHIFT)
            && text(bind, "submap").is_empty()
        {
            theirs = true;
        }
    }
    if theirs { Chord::Theirs } else { Chord::Free }
}

/// This binary with the flag, quoted for the shell Hyprland runs it through.
fn read_screen_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.to_str()?;
    if exe.contains("]==]") {
        return None;
    }
    Some(format!("'{}' --read-screen", exe.replace('\'', "'\\''")))
}

fn hyprland_binds() -> Option<Vec<serde_json::Value>> {
    let mut command = std::process::Command::new("hyprctl");
    command.args(["binds", "-j"]);
    let out = super::timed_output(command, super::IPC_BUDGET).ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Runs `hyprctl`, and says whether it both ran and did not answer with an
/// error — which it prints on stdout with a successful exit.
fn hyprctl_ok(args: &[&str]) -> bool {
    let mut command = std::process::Command::new("hyprctl");
    command.args(args);
    super::timed_output(command, super::IPC_BUDGET)
        .is_ok_and(|out| out.status.success() && !out.stdout.starts_with(b"Error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_binding_is_recognised_and_a_users_own_is_left_alone() {
        let bind = |key: &str, modmask: i64, description: &str| {
            serde_json::json!({
                "key": key, "modmask": modmask, "submap": "",
                "description": description, "dispatcher": "__lua", "arg": "7",
            })
        };
        assert_eq!(classify(&[bind("E", 64, "Editor")]), Chord::Free);
        assert_eq!(classify(&[bind("E", 5, "Something else")]), Chord::Theirs);
        assert_eq!(
            classify(&[bind("E", 5, READ_SCREEN_DESCRIPTION)]),
            Chord::Ours
        );
        let legacy = serde_json::json!({
            "key": "E", "modmask": 5, "submap": "", "description": "",
            "dispatcher": "exec", "arg": "'/opt/bubbleTranslate' --read-screen",
        });
        assert_eq!(classify(&[legacy]), Chord::Ours);
    }

    #[test]
    fn every_gated_key_has_a_pair_of_keysyms() {
        for key in TriggerKey::ALL {
            match key {
                TriggerKey::Always => assert!(keysyms(*key).is_none()),
                other => {
                    let pair = keysyms(*other).expect("a real key needs keysyms");
                    assert_ne!(pair[0], pair[1], "left and right are different keys");
                }
            }
        }
    }

    /// Answers honestly rather than guessing when there is no compositor.
    #[test]
    fn no_compositor_means_no_answer() {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            // On a Hyprland session this is a live query, and what it returns
            // depends on whether a key happens to be down. Either answer is
            // correct; what matters is that it is not a panic.
            let _ = key_held(TriggerKey::Shift);
            return;
        }
        assert_eq!(key_held(TriggerKey::Shift), None);
    }
}
