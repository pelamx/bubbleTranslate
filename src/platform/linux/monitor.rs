//! Noticing that a selection just finished.
//!
//! Nothing here has to guess *what* was selected, the way the macOS tap does:
//! the desktop publishes a primary selection when, and only when, the user has
//! selected something, so the text that arrives is already the answer. What is
//! left to work out is *when the user is done*, because a selection is
//! published as it grows — drag a sentence out and a toolkit sends one change
//! per word — and translating the first two of them would put a bubble over
//! text still being swept.
//!
//! Two filters answer that, in the order of how much they know:
//!
//!   * on X11, waiting for the mouse button to come up. That is the gesture
//!     ending, not an inference about it, and no delay guesses at it.
//!   * on X11, whether the trigger key was held while it happened — the gate
//!     that keeps an ordinary selection from becoming a translation. It is
//!     read from the same pointer query the drag uses, and it cannot be read
//!     at all on Wayland, where no protocol reports the keyboard to an
//!     unfocused client.
//!   * everywhere, the engine's settle window, which now waits for the
//!     selection to stop changing rather than counting from its first sign.
//!     On Wayland it is the only filter there can be: no protocol will say
//!     whether a button is down, so quiet is the only evidence available.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use crate::config::TriggerKey;
use crate::platform::Trigger;

use super::{Backend, backend, capture, compositor, cursor, evdev, wayland, x11};

/// How long a held button may hold the bubble off. Longer than any sweep of a
/// paragraph, short enough that a button held for some other reason entirely
/// does not silence the translator.
const MAX_DRAG: Duration = Duration::from_secs(10);

/// Whether a copy should be treated like a selection.
///
/// Read by the backends, which see both and would otherwise ignore the
/// clipboard entirely.
static WATCH_CLIPBOARD: AtomicBool = AtomicBool::new(false);

pub fn set_watch_clipboard(watch: bool) {
    WATCH_CLIPBOARD.store(watch, Ordering::Relaxed);
}

pub(super) fn watching_clipboard() -> bool {
    WATCH_CLIPBOARD.load(Ordering::Relaxed)
}

/// Which key has to be held for a selection to count, as an index into
/// [`TriggerKey::ALL`]-style ordering below.
///
/// An atomic rather than the config mutex because the monitor thread reads it
/// on the path a selection arrives on, and that path must never be able to
/// wait on the UI.
static TRIGGER_KEY: AtomicU8 = AtomicU8::new(0);

fn encode(key: TriggerKey) -> u8 {
    match key {
        TriggerKey::Always => 0,
        TriggerKey::Shift => 1,
        TriggerKey::Ctrl => 2,
        TriggerKey::Alt => 3,
        TriggerKey::Super => 4,
    }
}

fn decode(raw: u8) -> TriggerKey {
    match raw {
        1 => TriggerKey::Shift,
        2 => TriggerKey::Ctrl,
        3 => TriggerKey::Alt,
        4 => TriggerKey::Super,
        _ => TriggerKey::Always,
    }
}

pub fn set_trigger_key(key: TriggerKey) {
    TRIGGER_KEY.store(encode(key), Ordering::Relaxed);
}

fn trigger_key() -> TriggerKey {
    decode(TRIGGER_KEY.load(Ordering::Relaxed))
}

/// Whether a held key can be checked at all on this session.
///
/// Three ways to say yes. X11 carries the modifiers on the same query the drag
/// uses. Where it will not answer — every Wayland session — a compositor with
/// its own IPC may still be willing to say, and Hyprland is; failing that the
/// keyboard is read from `/dev/input`, which needs a permission the desktop
/// does not grant by default. The setting stays offered either way, because
/// the same config file follows the user to another session and to a Mac, but
/// the interface says plainly when it is not in force.
pub fn trigger_key_enforced() -> bool {
    matches!(backend(), Backend::X11Primary) || compositor::can_answer() || evdev::available()
}

/// Why the trigger key is not being enforced, for the interface to show.
///
/// `None` when it is. The reason is always the same shape — something will not
/// say what the keyboard is doing — but what to do about it differs, and only
/// this one has an answer the user can act on.
pub fn trigger_key_blocked() -> Option<String> {
    if trigger_key_enforced() {
        return None;
    }
    Some(match evdev::start_error() {
        Some(reason) => reason,
        None => "this session will not report the keyboard".to_string(),
    })
}

/// Whether the trigger key is held, asked of whichever source can answer.
///
/// X11 first: it is a question to a server we are already talking to. Then the
/// compositor, for the ones that will answer. `/dev/input` is the last resort,
/// and the only one that asks the user for anything.
fn key_held(key: TriggerKey) -> Option<bool> {
    if matches!(backend(), Backend::X11Primary)
        && let Some(held) = x11::modifier_held(key)
    {
        return Some(held);
    }
    // Before the device nodes, because it needs no permission at all: a
    // compositor that will answer is strictly better than one the user had to
    // work around.
    if let Some(held) = compositor::key_held(key) {
        return Some(held);
    }
    evdev::held(key)
}

/// Nothing to pause.
///
/// On macOS this guards against the bubble triggering a capture of the app
/// behind it, because the capture there can synthesize a copy into whatever is
/// frontmost. Here a trigger *is* a change to the primary selection, and
/// nothing the user does inside the bubble changes it — egui does not own the
/// selection, only the clipboard.
///
/// Left as a no-op rather than removed from the interface, because the flag it
/// would set is fed by the UI's repaint loop: while the bubble is hidden that
/// loop idles for an hour at a time, so a `true` written just before it went
/// quiet would never be cleared, and the monitor would stay paused for good.
pub fn set_paused(_paused: bool) {}

/// Starts the watch on a dedicated thread and returns immediately.
///
/// A session that cannot be watched is not an error to fail startup over: the
/// main window's translate box and the command line still work, and
/// [`super::capture::readiness`] is what tells the user why the bubble is
/// quiet.
pub fn spawn(on_trigger: impl Fn(Trigger) + Send + 'static) -> std::io::Result<()> {
    let backend = backend().clone();
    // The last resort only. Reading input devices is a privilege, so it is
    // taken only where neither the X server nor the compositor will answer —
    // asking for it on a session that does not need it would be asking for
    // nothing.
    if !matches!(backend, Backend::X11Primary) && !compositor::can_answer() {
        evdev::ensure_started();
    }
    std::thread::Builder::new()
        .name("selection-monitor".into())
        .spawn(move || {
            let dragging = matches!(backend, Backend::X11Primary);
            let handler = move |text: String, from_clipboard: bool| {
                // Latched before anything is decided, so a selection the gate
                // turns away is still the one the hotkey translates. It costs
                // a string either way, and the engine only reads it when a
                // trigger actually goes out.
                capture::latch(text);
                let key = trigger_key();
                // A copy is already a deliberate gesture with a key in it, so
                // it is not asked for a second one. Gating it would turn
                // "translate what I copy" into a two-modifier chord.
                // A copy is already a deliberate gesture with a key in it, so
                // it is not asked for a second one, and a session where
                // nothing will report the keyboard cannot gate anything.
                let gated = !from_clipboard && key != TriggerKey::Always && trigger_key_enforced();
                // Asked before the wait as well as during it: a keyboard
                // selection has no drag to poll through, and the key is let go
                // the instant the gesture ends.
                let mut held = gated && key_held(key).unwrap_or(true);
                // Nothing is worth reporting until the gesture is over: the
                // selection is still growing, and the pointer is not yet where
                // the user means to leave it. Waiting here rather than in the
                // engine is what puts the anchor at the end of the sweep.
                if dragging {
                    held |= x11::wait_while_dragging(MAX_DRAG, key);
                }
                // The drag wait is X11's; on Wayland the settle happens in the
                // engine, so the keyboard is asked once more on the way out.
                if gated && !held && !dragging {
                    held = key_held(key).unwrap_or(true);
                }
                if gated && !held {
                    crate::trace!("skip      {} was not held", key.label());
                    return;
                }
                on_trigger(Trigger {
                    at: cursor::position(),
                    // The X11 and Wayland readers below take the selection
                    // directly, so there is no copy of our own to tell apart
                    // from one the interface made.
                    clipboard_before: None,
                });
            };

            let outcome = match backend {
                Backend::WaylandDataControl => wayland::watch(handler),
                Backend::X11Primary => x11::watch(handler),
                Backend::Unavailable(reason) => Err(reason),
            };

            if let Err(err) = outcome {
                eprintln!("bubbleTranslate: not watching for selections — {err}");
            }
        })
        .map(|_| ())
}
