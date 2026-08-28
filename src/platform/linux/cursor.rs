//! Where the pointer is — when the session is willing to say.
//!
//! X11 answers this for anyone who asks. Wayland has no protocol for it at
//! all, which is a deliberate omission rather than an oversight: a client that
//! could track the pointer across the whole screen could watch the user work.
//! Compositors that expose their own IPC will still answer, so this asks the
//! ones that do and reports honestly when there is no one to ask.

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{Backend, backend};

/// The pointer's position in global display coordinates, top-left origin.
///
/// `None` is a normal answer, not a failure: the bubble anchors itself to a
/// screen corner instead, the way a notification would.
pub fn position() -> Option<(f64, f64)> {
    match backend() {
        Backend::X11Primary => super::x11::pointer(),
        // Under XWayland the X server's idea of the pointer is stale whenever
        // it is over a Wayland window, so asking X here would be worse than
        // not asking: it would confidently give a wrong answer.
        _ => compositor_pointer(),
    }
}

/// The size of the space [`position`] reports into.
///
/// Needed because that space is not the one the bubble is positioned in. A
/// toolkit works in its own points, derived from the display's DPI; a Wayland
/// compositor works in logical coordinates, derived from the output's scale.
/// On a scaled display the two disagree — by a factor of 1.17 on the machine
/// this was written on — and a position handed straight from one to the other
/// lands somewhere else entirely. Knowing both monitor sizes is what relates
/// them; see [`super::to_points`].
pub fn screen_size() -> Option<(f64, f64)> {
    /// Long enough that the shell-out is rare, short enough that plugging in a
    /// display does not leave the bubble misplaced for the rest of the session.
    const TTL: Duration = Duration::from_secs(5);

    static CACHE: Mutex<Option<(Instant, Option<(f64, f64)>)>> = Mutex::new(None);

    let mut cache = CACHE.lock().ok()?;
    if let Some((at, size)) = *cache {
        if at.elapsed() < TTL {
            return size;
        }
    }
    let size = match backend() {
        Backend::X11Primary => super::x11::screen_size(),
        _ => compositor_screen_size(),
    };
    *cache = Some((Instant::now(), size));
    size
}

fn compositor_screen_size() -> Option<(f64, f64)> {
    hyprland_monitor().map(|m| (m.width, m.height))
}

/// The scale the compositor draws the desktop at — 2 on a HiDPI laptop, 1 on
/// an ordinary display.
///
/// This is what every other application on the screen is sized by, so it is
/// what the bubble has to match to look like it belongs. See
/// [`crate::platform::preferred_zoom`].
pub fn compositor_scale() -> Option<f64> {
    if backend_is_x11() {
        // An X11 session has no compositor scale: the size of everything comes
        // from the server's DPI, which the toolkit already reads and the user
        // already chose. There is nothing to correct.
        return None;
    }
    hyprland_monitor().map(|m| m.scale)
}

fn backend_is_x11() -> bool {
    matches!(backend(), Backend::X11Primary)
}

struct MonitorInfo {
    /// Logical size: the output's pixels divided by its scale, which is the
    /// unit `cursorpos` reports in.
    width: f64,
    height: f64,
    scale: f64,
}

/// The monitor the pointer is on, as the compositor describes it.
///
/// `hyprctl monitors` reports each output in physical pixels alongside its
/// scale; logical size is the quotient, and that is the unit `cursorpos`
/// answers in.
fn hyprland_monitor() -> Option<MonitorInfo> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return None;
    }
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let monitors: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    let cursor = position();

    let read = |m: &serde_json::Value| -> Option<(f64, f64, MonitorInfo)> {
        let scale = m.get("scale")?.as_f64()?;
        if scale <= 0.0 {
            return None;
        }
        Some((
            m.get("x")?.as_f64()?,
            m.get("y")?.as_f64()?,
            MonitorInfo {
                width: m.get("width")?.as_f64()? / scale,
                height: m.get("height")?.as_f64()? / scale,
                scale,
            },
        ))
    };

    // The monitor under the pointer, since that is the space the pointer's
    // coordinates are being read in. With one display this is simply it.
    let mut first = None;
    for monitor in &monitors {
        let Some((x, y, info)) = read(monitor) else {
            continue;
        };
        if let Some((cx, cy)) = cursor {
            if cx >= x && cx < x + info.width && cy >= y && cy < y + info.height {
                return Some(info);
            }
        }
        if first.is_none() {
            first = Some(info);
        }
    }
    first
}

/// Asks the compositor directly, for the ones that have somewhere to ask.
///
/// Each of these is a small shell-out rather than a protocol binding, which is
/// the whole reason more of them can be added cheaply: a compositor that grows
/// a way to answer needs one arm here, not a dependency.
fn compositor_pointer() -> Option<(f64, f64)> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return hyprland_pointer();
    }
    None
}

/// `hyprctl cursorpos` prints `x, y` in layout coordinates, which is the same
/// space the bubble is positioned in.
fn hyprland_pointer() -> Option<(f64, f64)> {
    let out = Command::new("hyprctl").arg("cursorpos").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let (x, y) = text.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}
