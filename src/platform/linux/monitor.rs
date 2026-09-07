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
//!   * everywhere, the engine's settle window, which now waits for the
//!     selection to stop changing rather than counting from its first sign.
//!     On Wayland it is the only filter there can be: no protocol will say
//!     whether a button is down, so quiet is the only evidence available.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::platform::Trigger;

use super::{Backend, backend, capture, cursor, wayland, x11};

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
    std::thread::Builder::new()
        .name("selection-monitor".into())
        .spawn(move || {
            let dragging = matches!(backend, Backend::X11Primary);
            let handler = move |text: String| {
                // Nothing is worth reporting until the gesture is over: the
                // selection is still growing, and the pointer is not yet where
                // the user means to leave it. Waiting here rather than in the
                // engine is what puts the anchor at the end of the sweep.
                if dragging {
                    x11::wait_while_dragging(MAX_DRAG);
                }
                // The text has to be latched before the trigger goes out: the
                // engine reads it back the moment it stops debouncing.
                capture::latch(text);
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
