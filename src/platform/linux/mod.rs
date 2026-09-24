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

mod borrow;
mod compositor;
mod evdev;
mod ocr;
mod overlay;
mod portal;
mod reader;
mod screen;
mod wayland;
mod window;
mod x11;

use std::sync::OnceLock;

/// How long a compositor's IPC command may take to answer.
///
/// Every one of these is a local socket that answers in microseconds when the
/// compositor is healthy; the budget exists for the one that has stopped
/// answering. Every caller sits on the UI thread or on the repaint path, so
/// an unbounded wait here is a frozen interface, not a slow query.
pub(crate) const IPC_BUDGET: std::time::Duration = std::time::Duration::from_millis(750);

/// Runs a command to completion under a deadline, capturing its output.
///
/// `Command::output` has no timeout of its own, which is fine for commands
/// that always terminate and wrong for IPC with a process that can hang. The
/// pipes are drained on helper threads so a chatty child cannot block on a
/// full buffer while we wait; a child that overruns the budget is killed and
/// the call reports a timeout like any other failure.
pub(crate) fn timed_output(
    mut command: std::process::Command,
    budget: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
    let stderr = child
        .stderr
        .take()
        .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);

    let read_all = |pipe: Option<Box<dyn Read + Send>>| {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    };
    let stdout_task = std::thread::spawn(move || read_all(stdout));
    let stderr_task = std::thread::spawn(move || read_all(stderr));

    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the command did not finish in time",
                ));
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };

    Ok(std::process::Output {
        status,
        stdout: stdout_task.join().unwrap_or_default(),
        stderr: stderr_task.join().unwrap_or_default(),
    })
}

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

    // The lock is never held across the query: `cursor::position()` may
    // shell out to the compositor, and holding the mutex while it does would
    // turn one wedged query into a mutex every later caller waits on.
    let cached = CACHE.lock().ok().and_then(|cache| match *cache {
        Some((at, position)) if at.elapsed() < REFRESH => Some(position),
        _ => None,
    });
    let fresh = match cached {
        Some(position) => position,
        None => {
            let position = cursor::position();
            if let Ok(mut cache) = CACHE.lock() {
                *cache = Some((Instant::now(), position));
            }
            position
        }
    };

    // Nothing said where the pointer is, so nothing here can answer either;
    // the caller falls back to the toolkit.
    let at = fresh?;
    let (x, y) = to_points(at, monitor_points);
    Some(rect.contains(eframe::egui::pos2(x as f32, y as f32)))
}

/// Who to tell when the key that reads the screen is pressed.
static ON_REGION: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> = std::sync::Mutex::new(None);

/// Registers the callback for Ctrl+Shift+E, and starts listening for it.
///
/// Two ways in. On Hyprland the combination is bound in the compositor, which
/// swallows it — see [`compositor::ensure_read_screen_bind`]. Everywhere, it
/// is also read from `/dev/input`, the same reader the trigger key falls back
/// to, which works wherever the user is in the `input` group and is what
/// notices the press after a config reload has dropped the binding. Where
/// neither works, `bubbleTranslate --read-screen` is the same request, for a
/// desktop keybinding to run.
pub fn on_screen_region_request(ask: impl Fn() + Send + 'static) {
    if let Ok(mut slot) = ON_REGION.lock() {
        *slot = Some(Box::new(ask));
    }
    // Off the interface thread: it waits on the compositor, twice.
    let _ = std::thread::Builder::new()
        .name("read-screen-key".into())
        .spawn(|| {
            if compositor::ensure_read_screen_bind() != compositor::ReadScreenBind::None {
                KEY_BOUND.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            evdev::ensure_started();
        });
}

/// Whether Ctrl+Shift+E was bound in the compositor.
static KEY_BOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the main window shows about reading the screen here.
pub fn screen_reading(source_lang: &str) -> Option<crate::platform::ScreenReading> {
    let installed = ocr::languages();
    let engine = installed.is_some();
    let languages = installed.unwrap_or_default();

    // The language the user reads from decides which pack matters. With the
    // source left to detection there is no way to know, and English — the
    // pack most text on a screen needs — stands in until one is installed.
    let wanted = match source_lang {
        "auto" | "" => (languages.is_empty()).then_some("eng"),
        code => ocr::tesseract_code(code),
    };
    let missing_pack = wanted.filter(|pack| !languages.iter().any(|l| l == pack));
    let install = missing_pack.and_then(|pack| ocr::install_command(pack, !engine));

    let builtin = match reader::state() {
        reader::State::Absent => crate::platform::BuiltinReader::Absent,
        reader::State::Downloading(done) => crate::platform::BuiltinReader::Downloading(done),
        reader::State::Ready => crate::platform::BuiltinReader::Ready,
        reader::State::Failed(why) => crate::platform::BuiltinReader::Failed(why),
    };

    Some(crate::platform::ScreenReading {
        languages,
        builtin,
        install,
        missing_pack,
        key_heard: KEY_BOUND.load(std::sync::atomic::Ordering::Relaxed) || evdev::available(),
    })
}

/// What the keyboard reader does on Ctrl+Shift+E.
///
/// Passed on only when the compositor did not deliver it already: a binding
/// that was live ran `--read-screen` itself, and asking again would put the
/// overlay up twice.
pub(crate) fn read_screen_key_pressed() {
    if compositor::ensure_read_screen_bind() != compositor::ReadScreenBind::Live {
        ask_for_screen_region();
    }
}

/// Asks for the screen to be read, from the key or from a second process.
pub fn ask_for_screen_region() {
    match ON_REGION.lock() {
        Ok(slot) => match slot.as_ref() {
            Some(ask) => ask(),
            None => crate::trace!("region    nobody registered for the key"),
        },
        Err(_) => crate::trace!("region    the callback lock was poisoned"),
    }
}

/// Fetches the built-in reader's models in the background, for the window's
/// download button.
pub fn download_screen_reader() {
    reader::start_download();
}

/// Reads text out of a rectangle the user draws on the screen.
///
/// Three steps: a still of the screen is taken, the user draws a rectangle
/// over a dimmed copy of it, and Tesseract or the built-in reader reads what
/// is inside. `None` when any of them has nothing to give — no reader and no
/// network to fetch one, a desktop that will not be
/// read, a cancelled drag, a rectangle with no text in it. None of those is
/// worth a bubble saying so, the same as on Windows.
pub fn read_screen_region() -> Option<crate::platform::ScreenRead> {
    crate::trace!("region    asked to read the screen");

    // Before the screen is taken over: a machine that cannot answer should
    // not make the user draw a rectangle first. The first time, this is where
    // the built-in reader's models are fetched.
    if let Err(reason) = ocr::prepare() {
        crate::trace!("ocr       no reader: {reason}");
        return None;
    }

    let shot = match screen::grab() {
        Ok(shot) => shot,
        Err(err) => {
            crate::trace!("screen    {err}");
            return None;
        }
    };
    // A region the desktop's own screenshot interface already chose needs no
    // overlay, and has nowhere on screen to anchor the bubble to.
    let chosen = matches!(shot.placement, screen::Placement::Chosen);
    let (x, y, width, height) = if chosen {
        (0, 0, shot.frame.width, shot.frame.height)
    } else {
        overlay::select(&shot.frame, shot.placement)?
    };
    let region = shot.frame.crop(x, y, width, height);
    let text = ocr::recognize(&region, shot.scale())?;

    // Whitespace is what a rectangle drawn over a photograph comes back as.
    if text.trim().is_empty() {
        crate::trace!("ocr       nothing readable in the region");
        return None;
    }

    let scale = shot.scale();
    let region = (
        shot.origin.0 + f64::from(x) / scale,
        shot.origin.1 + f64::from(y) / scale,
        f64::from(width) / scale,
        f64::from(height) / scale,
    );
    let screen_bottom = shot.origin.1 + shot.size.1;
    Some(crate::platform::ScreenRead {
        capture: crate::platform::Capture {
            text,
            via: crate::platform::CaptureSource::Ocr,
        },
        at: (!chosen).then(|| bubble_anchor(region, screen_bottom)),
    })
}

/// How far below the region the bubble sits, in the pointer's units.
const REGION_GAP: f64 = 8.0;

/// How much room under the region counts as enough to put the bubble there.
/// A little more than the bubble's smallest height; being wrong is cosmetic,
/// because the bubble is kept on the screen either way.
const REGION_MIN_ROOM: f64 = 80.0;

/// How far inside the region the bubble sits when it has to go on top of it.
const REGION_INSET: f64 = 12.0;

/// Where the bubble goes for a region that was just read: underneath it, so
/// what was read stays in view beside its translation, or on top of it when
/// the region reaches the bottom of the screen and there is nowhere else.
fn bubble_anchor(region: (f64, f64, f64, f64), screen_bottom: f64) -> (f64, f64) {
    let (x, y, _, height) = region;
    let below = y + height + REGION_GAP;
    if below + REGION_MIN_ROOM <= screen_bottom {
        (x, below)
    } else {
        crate::trace!("bubble    no room under the region; placing it on top");
        (x + REGION_INSET, y + REGION_INSET)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bubble_goes_under_the_region_while_there_is_room() {
        assert_eq!(
            bubble_anchor((100.0, 100.0, 200.0, 50.0), 800.0),
            (100.0, 158.0)
        );
    }

    #[test]
    fn a_region_at_the_bottom_gets_the_bubble_on_top_of_it() {
        assert_eq!(
            bubble_anchor((100.0, 600.0, 200.0, 190.0), 800.0),
            (112.0, 612.0)
        );
    }
}
