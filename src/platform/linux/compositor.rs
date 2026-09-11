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

#[cfg(test)]
mod tests {
    use super::*;

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
