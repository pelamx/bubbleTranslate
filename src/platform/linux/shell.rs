//! Desktop shell integration: the tray icon and the way back to the window.
//!
//! The macOS build hides from the Dock and lives in the menu bar, which is
//! what makes closing the main window safe — the status item is the way back.
//! This is that status item for Linux, over the KDE/freedesktop
//! StatusNotifierItem specification, which is what a modern bar exposes a tray
//! through: waybar, quickshell, KDE, XFCE, and GNOME with the AppIndicator
//! extension. [`has_indicator`] reports whether one actually appeared, because
//! not every session has a host for it: on a session that has none, closing
//! the main window still quits rather than stranding a translator the user has
//! no way to reach.
//!
//! The icon is drawn here rather than looked up in the icon theme. A theme
//! lookup depends on the app having been installed, and on the bar searching
//! the same directories `install.sh` writes to; a pixmap is carried over D-Bus
//! and renders the same whether the binary was installed or run from
//! `target/release`.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip};

/// Set once the tray is registered and a host has picked it up. Read every
/// frame by the UI to decide what closing the main window means.
static INDICATOR: AtomicBool = AtomicBool::new(false);

/// Set once registration has stopped being attempted, either way. Distinct
/// from [`INDICATOR`] because "not yet" and "never" call for different
/// behaviour: an app told to start in the background waits on this before
/// deciding whether it is safe to have no window at all.
static SETTLED: AtomicBool = AtomicBool::new(false);

/// Set when "Open Bubble Translate" is chosen, cleared once the UI has acted
/// on it. A flag rather than a channel: the menu callback runs on ksni's own
/// thread, and the UI collects it on its next frame.
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Set when "Quit bubbleTranslate" is chosen. The tray thread cannot close the
/// viewport itself, so it asks and the UI loop does it.
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Needed to wake the render loop from a menu callback — the app idles at a
/// one-hour repaint interval while the bubble is hidden, so without this the
/// window would not appear until something else happened.
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();

/// Whether something outside the app's own windows can bring it back.
///
/// False until the tray has registered, which is what keeps "close means quit"
/// as the behaviour on a session with no tray host.
pub fn has_indicator() -> bool {
    INDICATOR.load(Ordering::SeqCst)
}

/// Whether [`has_indicator`] is a final answer yet.
///
/// False only for the first moments of a session, while the tray is still
/// being negotiated.
pub fn indicator_settled() -> bool {
    SETTLED.load(Ordering::SeqCst)
}

/// True exactly once per click on "Open Bubble Translate".
pub fn take_open_request() -> bool {
    OPEN_REQUESTED.swap(false, Ordering::SeqCst)
}

/// True exactly once per click on "Quit bubbleTranslate".
pub fn take_quit_request() -> bool {
    QUIT_REQUESTED.swap(false, Ordering::SeqCst)
}

fn request(flag: &AtomicBool, what: &str) {
    crate::trace!("tray: {what} requested");
    flag.store(true, Ordering::SeqCst);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

/// Installs the tray icon.
///
/// Registration is done on a thread of its own and retried, because at login
/// this app and the bar race: a bar that has not claimed
/// `org.kde.StatusNotifierWatcher` yet makes the first attempt fail on a
/// session that will support the tray perfectly well a second later.
pub fn install(ctx: eframe::egui::Context) {
    let _ = WAKE.set(ctx);
    std::thread::spawn(register);
}

/// How long to keep trying before deciding this session has no tray. Generous
/// enough to cover a bar starting alongside us, short enough that a session
/// which genuinely has no host settles on "close means quit" quickly.
const REGISTER_ATTEMPTS: u32 = 10;
const REGISTER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

fn register() {
    for attempt in 1..=REGISTER_ATTEMPTS {
        match BubbleTray.spawn() {
            Ok(handle) => {
                crate::trace!("tray: registered on attempt {attempt}");
                // The service runs on a thread of its own; this handle only
                // controls it, and it has to outlive this function for the
                // icon to stay up.
                std::mem::forget(handle);
                INDICATOR.store(true, Ordering::SeqCst);
                settle();
                return;
            }
            Err(err) if attempt == REGISTER_ATTEMPTS => {
                // Not fatal, and not worth a dialog: the app works exactly as
                // it did before the tray existed, closing the window quits.
                eprintln!(
                    "bubbleTranslate: no tray on this session ({err}); \
                     closing the window will quit."
                );
                settle();
            }
            Err(err) => {
                crate::trace!("tray: attempt {attempt} failed ({err}), retrying");
                std::thread::sleep(REGISTER_INTERVAL);
            }
        }
    }
}

/// Publishes the verdict and wakes the UI, which may be waiting on it before
/// it decides whether to show a window.
fn settle() {
    SETTLED.store(true, Ordering::SeqCst);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

struct BubbleTray;

impl ksni::Tray for BubbleTray {
    fn id(&self) -> String {
        "bubbleTranslate".into()
    }

    fn title(&self) -> String {
        "bubbleTranslate".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon::globe()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "bubbleTranslate".into(),
            description: "Select text anywhere to see a translation".into(),
            ..Default::default()
        }
    }

    /// A left click opens the window, the same as the menu item. Nothing here
    /// toggles: a tray icon that hides the window on a second click is
    /// indistinguishable from one that did not register the first.
    fn activate(&mut self, _x: i32, _y: i32) {
        request(&OPEN_REQUESTED, "open");
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Bubble Translate".into(),
                activate: Box::new(|_| request(&OPEN_REQUESTED, "open")),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit bubbleTranslate".into(),
                activate: Box::new(|_| request(&QUIT_REQUESTED, "quit")),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// No-ops, so the UI can call the same sequence it does on macOS.
///
/// Their macOS counterparts move the app between background and foreground
/// activation policies, a distinction X11 window managers do not have — a
/// window is raised by asking for it, which the UI does with a viewport
/// command right after calling these.
pub fn set_foreground(_visible: bool) {}

pub fn activate() {}

pub fn run_in_background() {}

/// Nothing to hook, so the UI's retry loop stops on the first frame. The macOS
/// counterpart intercepts the Finder's "reopen" event; the tray is what plays
/// that role here.
pub fn hook_reopen() -> bool {
    true
}

/// The tray icon, rasterized.
mod icon {
    use ksni::Icon;

    /// The sizes a bar is likely to ask for. Hosts pick the closest one and
    /// scale, so covering the common bar heights avoids a resample of a 16px
    /// glyph up to a 32px slot.
    const SIZES: [i32; 5] = [16, 22, 24, 32, 48];

    /// Matches the stroke colour of `linux/bubbleTranslate.svg`, and the light
    /// glyph a dark bar expects. Alpha comes from coverage.
    const INK: (u8, u8, u8) = (0xe8, 0xe8, 0xea);

    /// Samples per axis, per pixel. The glyph is all thin curves, so this is
    /// what keeps them from breaking up at 16px.
    const SUPERSAMPLE: i32 = 4;

    /// Radius of the sphere, as a fraction of the icon. Leaves a pixel of air
    /// at the smallest size so the outline is not clipped by the edge.
    const RADIUS: f32 = 0.44;

    /// Half-width of the meridian, as a fraction of the radius.
    const MERIDIAN: f32 = 0.40;

    /// A globe: the outline, the meridian, and the parallels — the same figure
    /// the app icon draws, without the speech bubble around it, which is
    /// illegible at this size.
    pub fn globe() -> Vec<Icon> {
        SIZES.iter().map(|&size| render(size)).collect()
    }

    fn render(size: i32) -> Icon {
        // ARGB32 in network byte order, which is what the SNI specification
        // asks for: one u32 per pixel, big-endian, alpha first.
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        let (r, g, b) = INK;

        for y in 0..size {
            for x in 0..size {
                let alpha = coverage(x, y, size);
                // Premultiplied, so a partly covered pixel does not read as a
                // bright fringe over a dark bar.
                data.push(alpha);
                data.push(mul(r, alpha));
                data.push(mul(g, alpha));
                data.push(mul(b, alpha));
            }
        }

        Icon {
            width: size,
            height: size,
            data,
        }
    }

    fn mul(channel: u8, alpha: u8) -> u8 {
        ((channel as u16 * alpha as u16) / 255) as u8
    }

    /// How much of this pixel the glyph covers, by supersampling.
    fn coverage(x: i32, y: i32, size: i32) -> u8 {
        let mut hits = 0;
        let step = 1.0 / SUPERSAMPLE as f32;
        for sy in 0..SUPERSAMPLE {
            for sx in 0..SUPERSAMPLE {
                let px = x as f32 + (sx as f32 + 0.5) * step;
                let py = y as f32 + (sy as f32 + 0.5) * step;
                if on_glyph(px, py, size) {
                    hits += 1;
                }
            }
        }
        ((hits * 255) / (SUPERSAMPLE * SUPERSAMPLE)) as u8
    }

    /// The stroke width, in whole pixels.
    ///
    /// Rounded rather than left fractional because a line half a pixel wide is
    /// not drawn thin, it is drawn grey: the coverage is spread over two rows
    /// and neither reaches full alpha. One pixel is the floor — below it the
    /// whole figure fades.
    fn stroke(size: i32) -> f32 {
        ((size as f32 * 0.055).round() as i32).max(1) as f32
    }

    /// Which parallels to draw at this size.
    ///
    /// A 16px slot cannot hold three of them and the meridian as well — the
    /// gaps between them come out under a pixel and the sphere fills in solid.
    /// Dropping to the equator alone is what keeps the small icon a globe
    /// rather than a disc, the same thing an icon theme does by shipping
    /// separate artwork per size.
    fn parallels(size: i32) -> &'static [f32] {
        if size >= 22 {
            &[-0.54, 0.0, 0.54]
        } else {
            &[0.0]
        }
    }

    /// Whether a point falls on one of the globe's strokes.
    ///
    /// The curves are tested by distance, which is what makes the supersampled
    /// result look drawn rather than stepped. The parallels are snapped to
    /// whole pixel rows instead, because they are the strokes a viewer reads
    /// the shape from and a straight line has no excuse to be blurry.
    fn on_glyph(px: f32, py: f32, size: i32) -> bool {
        let extent = size as f32;
        let center = extent / 2.0;
        let radius = extent * RADIUS;
        let stroke = stroke(size);
        let half = stroke / 2.0;

        let x = px - center;
        let y = py - center;

        // The outline.
        if ((x * x + y * y).sqrt() - radius).abs() <= half {
            return true;
        }

        // Everything else is drawn inside the sphere only, so the parallels
        // stop at the outline rather than running past it.
        if x * x + y * y >= radius * radius {
            return false;
        }

        // The meridian: a narrow ellipse. Distance to an ellipse has no closed
        // form, so this is the implicit function divided by its gradient —
        // first-order correct, which at a one-pixel stroke is exact enough.
        let rx = radius * MERIDIAN;
        let ry = radius;
        let f = (x * x) / (rx * rx) + (y * y) / (ry * ry) - 1.0;
        let gx = 2.0 * x / (rx * rx);
        let gy = 2.0 * y / (ry * ry);
        let grad = (gx * gx + gy * gy).sqrt();
        if grad > 0.0 && (f / grad).abs() <= half {
            return true;
        }

        // The equator, and the two parallels either side of it at larger
        // sizes, at the same latitudes as the app icon.
        parallels(size).iter().any(|lat| {
            let top = (center + lat * radius - half).floor();
            py >= top && py < top + stroke
        })
    }
}
