//! Noticing that a selection just finished.
//!
//! There is no gesture filtering here, and that absence is the point. On macOS
//! the tap has to guess when a selection is *complete* — drags past a
//! threshold, double and triple clicks — because guessing wrong means posting
//! a stray Cmd+C into whatever app is in front. Here the desktop itself
//! decides: a primary selection changes when, and only when, the user has
//! selected something. What arrives is already the answer.
//!
//! What remains is the debounce, which the engine does: X11 clients commonly
//! re-assert ownership as a drag grows, so a single sweep of the mouse can
//! land several changes in a row.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::platform::Trigger;

use super::{Backend, backend, capture, cursor, wayland, x11};

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
            let handler = move |text: String| {
                // The text has to be latched before the trigger goes out: the
                // engine reads it back the moment it stops debouncing.
                capture::latch(text);
                on_trigger(Trigger {
                    at: cursor::position(),
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
