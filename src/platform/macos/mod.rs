//! macOS: the Accessibility API for reading selections, a `CGEventTap` for
//! noticing them, and a status item for the menu bar.

pub mod capture;
pub mod monitor;
pub mod ocr;
pub mod shell;

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The page that grants screen recording, addressed directly.
///
/// The same reasoning as the Accessibility deep link in `capture.rs`: the
/// switch is four levels into System Settings, and naming the path is a
/// scavenger hunt rather than instructions.
pub const SCREEN_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Reads text out of a rectangle the user draws on the screen.
///
/// The rectangle is drawn with `screencapture -i`, the crosshair every Mac
/// user already knows. It brings multi-display, window mode on the space bar
/// and Escape-to-cancel with it; an overlay of our own would be a worse copy
/// of the same thing.
///
/// `None` for every way this comes to nothing — the permission withheld, the
/// drag cancelled, the region holding no text. None of them is worth a bubble:
/// the crosshair vanishing already said the request was dropped.
pub fn read_screen_region() -> Option<crate::platform::ScreenRead> {
    crate::trace!("region    asked to read the screen");

    // Asked before the crosshair rather than after, for the same reason
    // Windows checks its language list first: making someone draw a rectangle
    // and only then admitting nothing can come of it is a worse way to say so.
    if !granted() {
        // Asking opens System Settings and puts the app in the list, so there
        // is a switch to flip. It happens here and not at startup: the
        // permission belongs to this feature, and someone who never reads a
        // region should never be asked for it. macOS only hands the grant to a
        // fresh launch, so this run will still come back false.
        crate::trace!("region    no screen-recording permission; asking for it");
        core_graphics::access::ScreenCaptureAccess.request();
        shell::open_url(SCREEN_SETTINGS);
        return None;
    }
    if !ocr::available() {
        crate::trace!("ocr       no recognition language installed");
        return None;
    }

    // Anything recorded before now belongs to an earlier gesture.
    monitor::forget_last_drag();

    let shot = Shot::new();
    let out = Command::new("/usr/sbin/screencapture")
        .arg("-i") // interactive: drag a box, space toggles window mode
        .arg("-x") // no shutter sound; this is not a screenshot being kept
        .arg(&shot.0)
        .output()
        .ok()?;
    if !out.stderr.is_empty() {
        crate::trace!(
            "region    screencapture said {:?}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // Escape writes no file, and neither does Control-drag, which sends the
    // shot to the clipboard instead. Both mean the user changed their mind.
    if !shot.0.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        crate::trace!("region    nothing captured; the drag was given up");
        return None;
    }

    let source_lang = crate::config::Config::load().source_lang;
    let text = ocr::recognize(&shot.0, &source_lang)?;
    // The picture goes now, before the translation is even requested: it is a
    // picture of the user's screen and has no reason to outlive the words.
    drop(shot);

    // Whitespace is what a rectangle drawn over a photograph comes back as.
    if text.trim().is_empty() {
        crate::trace!("ocr       nothing readable in the region");
        return None;
    }

    Some(crate::platform::ScreenRead {
        capture: crate::platform::Capture {
            text,
            via: crate::platform::CaptureSource::Ocr,
        },
        at: monitor::take_last_drag().and_then(bubble_anchor),
    })
}

/// Registers the callback for Cmd+Shift+E.
pub fn on_screen_region_request(ask: impl Fn() + Send + 'static) {
    monitor::on_region_request(ask);
}

/// Whether screen recording has been granted, asked without prompting.
fn granted() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

/// How far below the region the bubble sits, in display points.
const REGION_GAP: f64 = 12.0;

/// Where the bubble goes for a region that was just read.
///
/// Outside it, underneath, so the rectangle the user drew stays visible beside
/// the translation of it — which is the whole point when what was read is a
/// picture and they want to compare the two.
///
/// A region with no room underneath is one that fills the screen, and there is
/// nowhere outside it left to go. Then the bubble goes *on* it, inset from the
/// corner so it sits inside rather than straddling the edge. Covering part of
/// what was read is the lesser loss: the reading has already happened.
fn bubble_anchor((x, y, _w, h): (f64, f64, f64, f64)) -> Option<(f64, f64)> {
    // Only the point below is chosen here. What happens when there is no room
    // for it is already settled one layer up: the bubble's own placement flips
    // it above the anchor when it would fall off the bottom, and clamps it to
    // the display either way -- so naming the screen's bounds again here would
    // be a second, worse copy of that.
    Some((x, y + h + REGION_GAP))
}

/// The screenshot on disk, deleted when this goes out of scope.
///
/// A picture of the user's screen should not outlive the sentence it produced,
/// and tying it to a value means every path out — the early returns and a
/// panic included — takes it along.
struct Shot(PathBuf);

impl Shot {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // The pid keeps two copies of the app apart, the counter keeps
        // successive reads apart; `ipc.rs` names its socket the same way.
        Self(std::env::temp_dir().join(format!(
            "bubbleTranslate-region-{}-{}.png",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        )))
    }
}

impl Drop for Shot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
