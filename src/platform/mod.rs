//! Where bubbleTranslate meets the desktop it is running on.
//!
//! Everything above this module — the providers, the engine, the bubble — is
//! the same code everywhere. Below it, three questions get answered in
//! whatever way the system underneath allows:
//!
//!   * what text is selected right now
//!   * when did a selection just finish
//!   * where is the pointer
//!
//! macOS answers all three with one set of APIs, gated behind a single
//! permission. Linux has no single answer — see [`linux`] for how the session
//! type and the compositor's protocols decide which route is taken.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{capture, monitor, shell};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{capture, monitor, shell};

/// How the selected text was obtained.
///
/// Worth carrying into the UI: a capture that borrowed the clipboard behaves
/// differently enough — it is slower, and it disturbs something the user owns
/// — that the bubble says so while it waits.
// Each platform constructs the subset it can produce; the others stay
// meaningful as a description of where text came from, and the bubble reads
// all of them.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    /// Asked the focused UI element what it has selected.
    Accessibility,
    /// Went through the clipboard, by synthesizing a copy.
    Clipboard,
    /// Read the desktop's primary selection, which selecting text fills in by
    /// itself. Nothing is synthesized and nothing the user owns is touched.
    PrimarySelection,
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub text: String,
    pub via: CaptureSource,
}

/// A selection gesture that just finished: the cue to capture and translate.
#[derive(Debug, Clone, Copy)]
pub struct Trigger {
    /// Where to anchor the bubble, in global display points with a top-left
    /// origin.
    ///
    /// `None` when the session will not say where the pointer is. Wayland has
    /// no protocol for asking — it is a deliberate omission, not a gap — so on
    /// a compositor without a private one this is genuinely unknowable and the
    /// bubble falls back to a screen corner.
    pub at: Option<(f64, f64)>,
}

/// Whether the pointer is inside `rect`, which is given in the same
/// coordinate space the bubble is positioned in.
///
/// `None` means "ask the toolkit instead": on a system where the bubble's
/// window reliably gets enter and leave events, egui already knows this and
/// asking the system again would be a needless round trip. It is only where
/// those events go missing that the question has to be put to the system.
#[cfg(target_os = "macos")]
pub fn pointer_over(_rect: egui::Rect, _monitor_points: Option<egui::Vec2>) -> Option<bool> {
    // AppKit delivers enter and leave to the bubble even though it never takes
    // focus, so egui's own pointer state is right here.
    None
}

/// Converts a [`Trigger`] anchor into the toolkit's points.
///
/// Identity on macOS, where AppKit's points and egui's are the same unit. Not
/// identity on Linux, where the pointer arrives in the compositor's logical
/// coordinates and a scaled display makes those a different size.
#[cfg(target_os = "macos")]
pub fn to_points(at: (f64, f64), _monitor_points: Option<egui::Vec2>) -> (f64, f64) {
    at
}

/// The zoom that makes the app's text match the rest of the desktop.
///
/// `None` on macOS, where the window server gives every application the same
/// scale and there is nothing to reconcile.
#[cfg(target_os = "macos")]
pub fn preferred_zoom(_native_pixels_per_point: f32) -> Option<f32> {
    None
}

/// Declares the bubble a notification window, so the window manager leaves it
/// undecorated, out of the taskbar, and unfocused.
///
/// Nothing to do on macOS: the bubble's viewport is already borderless and
/// non-activating, and AppKit has no equivalent hint to set.
#[cfg(target_os = "macos")]
pub fn mark_as_notification(_window: u32) {}

/// Asks for the bubble to appear on every workspace.
///
/// Nothing to do on macOS: a non-activating panel already shows on whichever
/// Space is in front.
#[cfg(target_os = "macos")]
pub fn keep_on_all_workspaces() -> bool {
    true
}

#[cfg(target_os = "linux")]
pub use linux::{
    keep_on_all_workspaces, mark_as_notification, pointer_over, preferred_zoom, to_points,
};

/// Whether selections can actually be watched here, and what to tell the user
/// when they cannot.
///
/// The reason is always platform-specific — a permission switch on macOS, a
/// missing protocol on Linux — so the text travels with the verdict rather
/// than being hardcoded into the UI.
pub struct Readiness {
    pub ok: bool,
    /// Shown in the main window's status panel. One or two sentences,
    /// including what to do about it.
    pub detail: String,
    /// The same problem in one line, for the bubble's settings panel.
    pub summary: String,
}

impl Readiness {
    pub fn ready() -> Self {
        Self {
            ok: true,
            detail: String::new(),
            summary: String::new(),
        }
    }

    pub fn blocked(summary: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            summary: summary.into(),
        }
    }
}
