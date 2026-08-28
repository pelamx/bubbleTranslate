//! Linux: one binary, several desktops, and no single way to read a selection.
//!
//! Three facts shape everything below.
//!
//! **Selections.** X11 has a primary selection any client may read, and
//! selecting text fills it in — there is nothing to capture, only something to
//! notice. Wayland deliberately gates the same thing behind keyboard focus,
//! which a background translator never has, and offers `wlr-data-control` as
//! the way out for clipboard managers. Most compositors implement it; GNOME
//! does not, and on GNOME's Wayland session no application can read another's
//! selection at all. That is a policy of the platform, not a gap here, so the
//! app says so plainly instead of appearing to work.
//!
//! **The pointer.** Wayland has no protocol for asking where the pointer is,
//! again on purpose. Compositors with their own IPC will answer — Hyprland
//! does — and where none will, the bubble goes to a corner of the screen
//! rather than to the cursor.
//!
//! **The window.** A Wayland toplevel cannot choose its own position, so the
//! interface is built as an X11 client on every session: native on X11,
//! XWayland on Wayland. That is what lets one binary put the bubble at the
//! cursor everywhere. Note that this is only about *drawing*: reading the
//! selection still goes over Wayland, because an X11 client cannot read the
//! bridged selection without focus either.

pub mod capture;
pub mod cursor;
pub mod monitor;
pub mod shell;

mod wayland;
mod window;
mod x11;

use std::sync::OnceLock;

/// How this session gets its selections. Decided once, at first use.
#[derive(Debug, Clone)]
pub enum Backend {
    /// Wayland with `wlr-data-control`: the compositor pushes every selection
    /// at us as it happens, focus or no focus.
    WaylandDataControl,
    /// A real X11 session: the primary selection, watched with XFixes.
    X11Primary,
    /// Nowhere to read from. Carries the reason, which is shown to the user —
    /// there is nothing they can enable to fix it, so the honest thing is to
    /// explain and fall back to typing into the main window.
    Unavailable(String),
}

/// True when this is a Wayland session, whatever the window happens to be
/// drawn through.
pub fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// The zoom that makes the app's text the same size as the rest of the desktop.
///
/// Under XWayland a scaled display is described twice and the two do not
/// agree. The compositor draws everything at its output scale — 2 on a HiDPI
/// laptop — while the X server hands the toolkit a DPI of its own, and on the
/// machine this was written on that works out to 2.33 pixels per point. The
/// result is an application whose text is 17% larger than every other window
/// on screen, for no reason the user can see.
///
/// Pinning the zoom so that one point is one compositor logical pixel fixes
/// that, and has a second effect worth having: the pointer's coordinates
/// arrive in exactly that unit, so placing the bubble at the cursor stops
/// needing a conversion at all.
///
/// `None` on an X11 session, where the server's DPI is not a second opinion —
/// it is the setting, chosen by the user, and other applications follow it too.
pub use window::{keep_on_all_workspaces, mark_as_notification};

pub fn preferred_zoom(native_pixels_per_point: f32) -> Option<f32> {
    if native_pixels_per_point <= 0.0 {
        return None;
    }
    let scale = cursor::compositor_scale()? as f32;
    let zoom = scale / native_pixels_per_point;
    // A ratio far from 1 means one of the two numbers is not what it claims;
    // leaving the toolkit alone is better than trusting it.
    (0.25..=4.0).contains(&zoom).then_some(zoom)
}

/// How many of the toolkit's points one unit of the pointer's space is worth.
///
/// The two disagree whenever a display is scaled: the toolkit derives its
/// points from the X server's DPI, the compositor derives its logical
/// coordinates from the output's scale, and nothing makes those agree. Rather
/// than trying to predict the factor, it is measured — the same monitor,
/// described in both units, is the whole conversion.
///
/// Falls back to 1.0, which is exactly right on an unscaled display and is the
/// only sane guess when either size is unknown.
///
/// The monitor is the one the bubble's window is on, which is the right answer
/// on a single display and on a multi-monitor layout with a uniform scale. A
/// mixed-DPI layout would need per-monitor conversion, and the bubble can land
/// off by that difference on the odd monitor out.
fn points_per_unit(monitor_points: Option<eframe::egui::Vec2>) -> f64 {
    let (Some(points), Some(units)) = (monitor_points, cursor::screen_size()) else {
        return 1.0;
    };
    if units.0 <= 0.0 || points.x <= 0.0 {
        return 1.0;
    }
    f64::from(points.x) / units.0
}

/// Converts a pointer position into the toolkit's points.
pub fn to_points(at: (f64, f64), monitor_points: Option<eframe::egui::Vec2>) -> (f64, f64) {
    let scale = points_per_unit(monitor_points);
    (at.0 * scale, at.1 * scale)
}

/// Whether the pointer is inside `rect`, which is in points.
///
/// Asked of the system rather than of egui because the bubble's window does
/// not get a reliable "pointer left" here — it never takes focus, and what
/// arrives is an enter and then silence. egui's pointer state therefore
/// latches the first time the pointer crosses the bubble and never clears,
/// which would pause the selection monitor for good and stop the bubble ever
/// hiding itself.
///
/// Throttled, because on Wayland the answer costs a round trip to the
/// compositor and this is asked while repainting.
pub fn pointer_over(
    rect: eframe::egui::Rect,
    monitor_points: Option<eframe::egui::Vec2>,
) -> Option<bool> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Slow enough to stay cheap, fast enough that moving off the bubble
    /// restarts its auto-hide countdown without a visible lag.
    const REFRESH: Duration = Duration::from_millis(150);

    static CACHE: Mutex<Option<(Instant, Option<(f64, f64)>)>> = Mutex::new(None);

    let mut cache = CACHE.lock().ok()?;
    let fresh = match *cache {
        Some((at, position)) if at.elapsed() < REFRESH => position,
        _ => {
            let position = cursor::position();
            *cache = Some((Instant::now(), position));
            position
        }
    };

    // Nothing said where the pointer is, so nothing here can answer either;
    // the caller falls back to the toolkit.
    let at = fresh?;
    let (x, y) = to_points(at, monitor_points);
    Some(rect.contains(eframe::egui::pos2(x as f32, y as f32)))
}

pub fn backend() -> &'static Backend {
    static BACKEND: OnceLock<Backend> = OnceLock::new();
    BACKEND.get_or_init(detect)
}

fn detect() -> Backend {
    if on_wayland() {
        // No falling through to X11 here even though XWayland is almost
        // certainly running: an X11 client on a Wayland session gets the
        // bridged selection only while it holds focus, so that path would
        // connect, watch, and never once fire.
        return match wayland::probe() {
            Ok(()) => {
                crate::trace!("backend: wayland data-control");
                Backend::WaylandDataControl
            }
            Err(reason) => {
                crate::trace!("backend: unavailable ({reason})");
                Backend::Unavailable(reason)
            }
        };
    }

    if std::env::var_os("DISPLAY").is_some() {
        return match x11::probe() {
            Ok(()) => {
                crate::trace!("backend: x11 primary selection");
                Backend::X11Primary
            }
            Err(reason) => Backend::Unavailable(reason),
        };
    }

    Backend::Unavailable("no display server: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
}
