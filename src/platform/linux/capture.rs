//! The selection, once something else has already read it.
//!
//! There is no capture step on Linux in the way there is on macOS. Both
//! backends deliver the *text* along with the news that a selection happened —
//! X11's primary selection and Wayland's data-control are both content, not
//! just a signal — so by the time the engine asks, the answer is already in
//! hand. This module is where it waits.
//!
//! That also means `allow_clipboard` has nothing to gate here: no copy is
//! synthesized, no keystroke is posted, and the user's clipboard is never
//! borrowed. The setting stays honest by simply not applying.

use std::sync::Mutex;

use crate::platform::{Capture, CaptureSource, Readiness};

use super::{Backend, backend, wayland};

/// The selection behind the trigger the engine has not consumed yet.
///
/// One slot rather than a queue on purpose: while the engine debounces, later
/// selections supersede earlier ones, and only the last is worth translating.
static LATCHED: Mutex<Option<String>> = Mutex::new(None);

/// Handed the text by whichever backend is watching, just before it fires the
/// trigger the engine will act on.
pub(super) fn latch(text: String) {
    *LATCHED.lock().unwrap() = Some(text);
}

pub fn selected_text(_allow_clipboard: bool) -> Option<Capture> {
    let text = LATCHED.lock().unwrap().take()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(Capture {
        text: trimmed.to_string(),
        via: CaptureSource::PrimarySelection,
    })
}

/// Whether this session can be watched at all.
///
/// Unlike the macOS permission, a `false` here is not something the user can
/// turn on. It means the compositor has no protocol for handing a selection to
/// an unfocused client, so the text says what still works rather than offering
/// a fix that does not exist.
pub fn readiness() -> Readiness {
    match backend() {
        Backend::WaylandDataControl | Backend::X11Primary => Readiness::ready(),
        Backend::Unavailable(reason) => Readiness::blocked(
            format!("Selections cannot be watched here — {reason}. Typing into the box above still works."),
            format!(
                "Selections cannot be watched on this desktop: {reason}. This is the \
                 compositor's policy, not a setting — GNOME's Wayland session, in \
                 particular, lets no application read another's selection. Translating \
                 text typed into the box above still works, as does the command line."
            ),
        ),
    }
}

/// Puts text on the clipboard, for the bubble's copy button.
///
/// Two routes, for the same reason the interface is an X11 client but the
/// selections are not: on Wayland, owning a clipboard through XWayland needs
/// the focus we do not have, so it goes over data-control instead. On X11 the
/// toolkit already does this correctly and there is nothing to add.
pub fn set_clipboard(ctx: &eframe::egui::Context, text: &str) {
    if super::on_wayland() {
        wayland::set_clipboard(text.to_string());
        return;
    }
    ctx.copy_text(text.to_string());
}
