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
//! type and the compositor's protocols decide which route is taken. Windows
//! answers the last two freely and the first not at all: nothing there
//! publishes a selection, so it has to be asked for at the moment the gesture
//! ends — see [`windows`].

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{capture, monitor, shell};
#[cfg(target_os = "macos")]
pub use macos::{on_screen_region_request, read_screen_region};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{capture, monitor, shell};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{capture, monitor, shell};

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
    /// Read off the screen itself, out of a rectangle the user drew.
    ///
    /// The one source that never asked an application anything, and so the
    /// only one that works on a picture, a video frame or a remote desktop —
    /// and the only one that can be wrong about what it read.
    Ocr,
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
    /// The clipboard's change count when the gesture began, where the platform
    /// can sample one.
    ///
    /// It is how the capture tells a selection the interface copied *itself*
    /// from the clipboard the user already had: only a clipboard that moved
    /// during this gesture is the selection. `None` for a gesture with no
    /// press to sample at, such as a shift- or select-all selection.
    pub clipboard_before: Option<isize>,
}

/// Where the pointer is, in the same space a [`Trigger`] anchor is given in.
///
/// Only the hotkey path asks: a selection's trigger carries its own anchor,
/// taken at the moment the gesture ended, and this is for the case where there
/// was no gesture to take one from.
///
/// A null event is the documented way to ask the window server for the cursor
/// without an event to read it off, and it answers in the top-left origin the
/// bubble is placed in.
#[cfg(target_os = "macos")]
pub fn cursor_position() -> Option<(f64, f64)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let point = CGEvent::new(source).ok()?.location();
    Some((point.x, point.y))
}

#[cfg(target_os = "linux")]
pub use linux::cursor::position as cursor_position;

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

/// Text read off the screen, and where to put the bubble that says it.
#[derive(Debug, Clone)]
pub struct ScreenRead {
    pub capture: Capture,
    /// Where to anchor the bubble, in the same space a [`Trigger`] anchor is
    /// given in.
    ///
    /// Chosen against the region rather than the pointer, because the pointer
    /// finished the drag somewhere arbitrary — at the far corner of whatever
    /// was being read, which is the one place the bubble must not cover.
    pub at: Option<(f64, f64)>,
}

/// What the main window can say about reading the screen on this system,
/// beyond how to do it.
///
/// Only Linux has anything to say: there the reading is done by a built-in
/// reader whose models are fetched on first use, or by Tesseract when the
/// user installed it for the letters the built-in one does not know. The
/// window shows which is ready, which languages Tesseract reads, and the
/// command that installs what is missing. Elsewhere the engine is part of the
/// system, and there is nothing to check.
pub struct ScreenReading {
    /// The languages Tesseract reads, as its own codes; empty when it is not
    /// installed or has none.
    pub languages: Vec<String>,
    /// Where the built-in reader stands.
    pub builtin: BuiltinReader,
    /// The one command that installs what is missing for the source language
    /// — Tesseract itself if it is absent, the language pack if only that is —
    /// or `None` when nothing is missing or this distribution's package names
    /// are not known.
    pub install: Option<String>,
    /// The source language's pack, when it is not installed.
    pub missing_pack: Option<&'static str>,
    /// Whether Ctrl+Shift+E itself reaches the app on this session. When it
    /// does not, the same thing is a command to bind to a key.
    pub key_heard: bool,
}

/// The built-in reader, which reads the screen on Linux when Tesseract is
/// not installed.
#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinReader {
    /// Its models have not been fetched yet.
    Absent,
    /// Being fetched; how far, from 0 to 1.
    Downloading(f32),
    Ready,
    /// The last attempt to fetch them failed, and why.
    Failed(String),
}

#[cfg(not(target_os = "linux"))]
pub fn screen_reading(_source_lang: &str) -> Option<ScreenReading> {
    None
}

/// Nothing to fetch where the system has its own reader.
#[cfg(not(target_os = "linux"))]
pub fn download_screen_reader() {}

/// Cuts the bubble's window to the shape of the card painted inside it.
///
/// Nothing to do wherever the bubble's window is transparent, which is
/// everywhere but Windows: there the card's own rounded corners are the only
/// edge there is, because nothing outside them is painted at all.
#[cfg(not(target_os = "windows"))]
pub fn shape_bubble() {}

/// How far above and left of the bubble its window has to be placed.
///
/// Nothing anywhere but Windows, where the bubble's window keeps a frame it
/// never shows; see [`windows::frame_offset`].
#[cfg(not(target_os = "windows"))]
pub fn frame_offset() -> (f32, f32) {
    (0.0, 0.0)
}

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
    ask_for_screen_region, download_screen_reader, keep_on_all_workspaces, mark_as_notification,
    on_screen_region_request, pointer_over, preferred_zoom, read_screen_region, screen_reading,
    to_points,
};

#[cfg(target_os = "windows")]
pub use windows::{
    cursor_position, frame_offset, keep_on_all_workspaces, mark_as_notification,
    on_screen_region_request, pointer_over, preferred_zoom, read_screen_region, shape_bubble,
    to_points,
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
    /// The page that grants what is missing, where the system has one to
    /// open.
    ///
    /// A sentence naming a path four levels into System Settings is not
    /// instructions, it is a scavenger hunt — and the person reading it has
    /// just installed a translator, not agreed to go looking. Where the fix
    /// is a switch the user has to tick, the window offers the page itself.
    pub fix: Option<Fix>,
}

/// A button the status panel can offer, and where it goes.
pub struct Fix {
    pub label: &'static str,
    pub url: &'static str,
}

impl Readiness {
    pub fn ready() -> Self {
        Self {
            ok: true,
            detail: String::new(),
            summary: String::new(),
            fix: None,
        }
    }

    /// Unused on Windows, which has nothing to block: reading another
    /// application's selection there needs no permission and no protocol that
    /// might be missing.
    #[allow(dead_code)]
    pub fn blocked(summary: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            summary: summary.into(),
            fix: None,
        }
    }

    /// Names the page that grants what is missing. Unused on Linux, where
    /// what is missing is a protocol the session does not speak — there is no
    /// switch to send anyone to.
    #[allow(dead_code)]
    pub fn fixable(mut self, label: &'static str, url: &'static str) -> Self {
        self.fix = Some(Fix { label, url });
        self
    }
}
