//! Choosing the rectangle to read, by dragging one over a dimmed screen.
//!
//! The same gesture as on Windows and in every screenshot tool: the screen
//! goes dim, the pointer becomes a crosshair, and what you drag over comes
//! back to full brightness. Let go and that is the region; Escape or the right
//! button and nothing happened.
//!
//! **What is on screen is a picture.** The overlay shows the still taken in
//! [`super::screen`], dimmed, and undims the part being selected by copying
//! that part of the undimmed still over it. Both copies live in the X server
//! as pixmaps, uploaded once, so following the pointer costs two server-side
//! copies per movement and no pixels cross the socket.
//!
//! **It is an X11 window, as the bubble is.** A Wayland surface cannot cover
//! the screen on its own say-so without a layer-shell protocol GNOME does not
//! have, and this binary is already an X11 client everywhere. Under XWayland
//! it asks the window manager to make it full-screen — the compositor places
//! it, and it lands on the screen that has focus, which is the one that was
//! just photographed. On an X11 session it places itself over the whole root
//! and takes the pointer and keyboard, since there may be no window manager
//! to ask.
//!
//! Like the Windows overlay, this blocks the thread it runs on until the drag
//! ends. The caller is the engine's worker, not the interface.

use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, GrabMode, GrabStatus, ImageFormat, InputFocus, PropMode, Rectangle, Setup,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::ocr::Frame;
use super::screen::Placement;

/// A rectangle in the still's own pixels: `(x, y, width, height)`.
pub type Selection = (u32, u32, u32, u32);

/// How long the overlay waits for a drag before giving up on its own.
///
/// It takes the whole screen, and a compositor that put it somewhere the user
/// cannot reach — or never showed it at all — must not leave the translator
/// waiting behind an invisible window forever.
const PATIENCE: Duration = Duration::from_secs(120);

/// The smallest side worth reading, in the overlay's pixels. Below this there
/// is no glyph, only the end of a click that was not meant as a drag.
const MIN_SIDE: i32 = 6;

/// How much of the screen's brightness survives the dimming, out of 256.
const DIM: u32 = 100;

/// The X keycode of Escape. X keycodes are the kernel's plus eight, and the
/// kernel's is 1 on every keyboard, whatever its layout says.
const ESCAPE: u8 = 9;

/// The crosshair in the X cursor font.
const XC_CROSSHAIR: u16 = 34;

/// Whether this server stores pixels of `depth` as four bytes, blue first —
/// the layout both the capture and [`Frame`] use. True on every little-endian
/// server with a 24- or 32-bit screen, which is every one this is built for;
/// asked rather than assumed so that anything else fails plainly instead of
/// painting garbage.
pub fn pixels_are_bgrx(setup: &Setup, depth: u8) -> bool {
    (depth == 24 || depth == 32)
        && setup
            .pixmap_formats
            .iter()
            .any(|format| format.depth == depth && format.bits_per_pixel == 32)
}

/// Shows `still` and blocks until a rectangle is drawn on it.
///
/// `None` when the user cancelled, let go without dragging, or the overlay
/// could not be shown. The rectangle comes back in the still's pixels.
pub fn select(still: &Frame, placement: Placement) -> Option<Selection> {
    match run(still, placement) {
        Ok(selection) => selection,
        Err(err) => {
            crate::trace!("overlay   {err}");
            None
        }
    }
}

struct Overlay<'a> {
    conn: RustConnection,
    window: u32,
    gc: u32,
    depth: u8,
    still: &'a Frame,
    /// The window's size, and the two pixmaps made for it.
    size: (u16, u16),
    bright: u32,
    dim: u32,
}

fn run(still: &Frame, placement: Placement) -> Result<Option<Selection>, String> {
    let (conn, screen_num) =
        x11rb::connect(None).map_err(|err| format!("could not reach the X server: {err}"))?;
    let screen = conn.setup().roots[screen_num].clone();
    let depth = screen.root_depth;
    if !pixels_are_bgrx(conn.setup(), depth) {
        return Err(format!("cannot draw depth-{depth} pixels"));
    }

    let (x, y, width, height, override_redirect) = match placement {
        Placement::Cover {
            x,
            y,
            width,
            height,
        } => (x, y, width, height, true),
        // A first guess only: the window manager decides, and the window is
        // repainted for whatever size it is given.
        Placement::Fullscreen => (
            0,
            0,
            still.width.min(u32::from(u16::MAX)) as u16,
            still.height.min(u32::from(u16::MAX)) as u16,
            false,
        ),
    };

    if !override_redirect {
        hyprland_rule();
    }

    let window = conn.generate_id().map_err(|e| e.to_string())?;
    let cursor = crosshair(&conn).unwrap_or(0);
    conn.create_window(
        depth,
        window,
        screen.root,
        x,
        y,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(screen.black_pixel)
            .override_redirect(u32::from(override_redirect))
            .cursor(cursor)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS,
            ),
    )
    .map_err(|e| e.to_string())?;
    describe(&conn, window, override_redirect)?;

    let gc = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_gc(
        gc,
        window,
        &CreateGCAux::new()
            .foreground(0x00ff_ffff)
            .line_width(2)
            .graphics_exposures(0),
    )
    .map_err(|e| e.to_string())?;

    let mut overlay = Overlay {
        conn,
        window,
        gc,
        depth,
        still,
        size: (0, 0),
        bright: 0,
        dim: 0,
    };
    // Uploaded before the window is shown, so it never appears black while
    // the pixels are still on their way.
    overlay.prepare((width, height))?;

    overlay.conn.map_window(window).map_err(|e| e.to_string())?;
    overlay.conn.flush().map_err(|e| e.to_string())?;
    if override_redirect {
        overlay.grab()?;
    }

    let result = overlay.drag();
    overlay.close();
    result
}

impl Overlay<'_> {
    /// Makes the two pictures the window is painted from, at its size.
    fn prepare(&mut self, size: (u16, u16)) -> Result<(), String> {
        if size == self.size || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.free_pixmaps();
        let fitted = self.still.resized(u32::from(size.0), u32::from(size.1));
        let mut dimmed = fitted.bgrx.clone();
        for level in &mut dimmed {
            *level = ((u32::from(*level) * DIM) >> 8) as u8;
        }
        self.bright = self.upload(&fitted.bgrx, size)?;
        self.dim = self.upload(&dimmed, size)?;
        self.size = size;
        // The server repaints from the dimmed copy on its own whenever part
        // of the window is exposed, so there is never a black flash to cover.
        self.conn
            .change_window_attributes(
                self.window,
                &ChangeWindowAttributesAux::new().background_pixmap(self.dim),
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Sends `pixels` to the server as a new pixmap, in as many requests as
    /// the server's size limit makes it take.
    fn upload(&self, pixels: &[u8], size: (u16, u16)) -> Result<u32, String> {
        let pixmap = self.conn.generate_id().map_err(|e| e.to_string())?;
        self.conn
            .create_pixmap(self.depth, pixmap, self.window, size.0, size.1)
            .map_err(|e| e.to_string())?;
        let row = usize::from(size.0) * 4;
        // The request header is 24 bytes; the rest of the limit is pixels.
        let rows_per_request = ((self.conn.maximum_request_bytes() - 24) / row).max(1);
        for (chunk, rows) in pixels.chunks(row * rows_per_request).enumerate() {
            let top = chunk * rows_per_request;
            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    pixmap,
                    self.gc,
                    size.0,
                    (rows.len() / row) as u16,
                    0,
                    top as i16,
                    0,
                    self.depth,
                    rows,
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(pixmap)
    }

    fn free_pixmaps(&mut self) {
        for pixmap in [self.bright, self.dim] {
            if pixmap != 0 {
                let _ = self.conn.free_pixmap(pixmap);
            }
        }
        self.bright = 0;
        self.dim = 0;
    }

    /// Takes the pointer and keyboard, for an overlay no window manager is
    /// managing. Retried briefly: a grab fails while another client's is still
    /// being let go, which is exactly the moment a key combination was pressed.
    fn grab(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let pointer = self
                .conn
                .grab_pointer(
                    true,
                    self.window,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    self.window,
                    0u32,
                    x11rb::CURRENT_TIME,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .is_some_and(|reply| reply.status == GrabStatus::SUCCESS);
            let keyboard = self
                .conn
                .grab_keyboard(
                    true,
                    self.window,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .is_some_and(|reply| reply.status == GrabStatus::SUCCESS);
            if pointer && keyboard {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("could not take the pointer and keyboard".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Follows the pointer until the drag ends one way or the other.
    fn drag(&mut self) -> Result<Option<Selection>, String> {
        let started = Instant::now();
        let mut anchor: Option<(i16, i16)> = None;
        let mut shown: Option<Rectangle> = None;
        let mut focused = false;

        loop {
            if started.elapsed() > PATIENCE {
                crate::trace!("overlay   nobody drew anything; giving up");
                return Ok(None);
            }
            let Some(event) = self.conn.poll_for_event().map_err(|e| e.to_string())? else {
                std::thread::sleep(Duration::from_millis(4));
                continue;
            };
            match event {
                Event::MapNotify(_) if !focused => {
                    // Escape only reaches a window that has the keyboard. A
                    // window manager normally hands it over to a new
                    // full-screen window by itself; asking costs nothing when
                    // it already has.
                    focused = true;
                    let _ = self.conn.set_input_focus(
                        InputFocus::PARENT,
                        self.window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.conn.flush();
                }
                Event::ConfigureNotify(configure) => {
                    self.prepare((configure.width, configure.height))?;
                    self.paint(None, shown);
                }
                Event::Expose(expose) if expose.count == 0 => self.paint(None, shown),
                Event::KeyPress(key) if key.detail == ESCAPE => return Ok(None),
                Event::ButtonPress(press) if press.detail == 3 => return Ok(None),
                Event::ButtonPress(press) if press.detail == 1 => {
                    anchor = Some((press.event_x, press.event_y));
                }
                Event::MotionNotify(motion) => {
                    if let Some(anchor) = anchor {
                        let now = between(anchor, (motion.event_x, motion.event_y));
                        self.paint(shown, Some(now));
                        shown = Some(now);
                    }
                }
                Event::ButtonRelease(release) if release.detail == 1 => {
                    let Some(anchor) = anchor else { continue };
                    let rect = between(anchor, (release.event_x, release.event_y));
                    if i32::from(rect.width) < MIN_SIDE || i32::from(rect.height) < MIN_SIDE {
                        crate::trace!("overlay   let go without dragging");
                        return Ok(None);
                    }
                    return Ok(Some(self.in_still(rect)));
                }
                _ => {}
            }
        }
    }

    /// Repaints what changed: the old selection goes back to dim, the new one
    /// comes up bright, and a frame is drawn around it.
    fn paint(&self, before: Option<Rectangle>, now: Option<Rectangle>) {
        let conn = &self.conn;
        match before {
            // The frame is drawn centred on the edge, so it reaches a pixel
            // outside the rectangle; that pixel has to be dimmed back too.
            Some(old) => {
                let old = grow(old, 2);
                let _ = conn.copy_area(
                    self.dim, self.window, self.gc, old.x, old.y, old.x, old.y, old.width,
                    old.height,
                );
            }
            None => {
                let _ = conn.copy_area(
                    self.dim,
                    self.window,
                    self.gc,
                    0,
                    0,
                    0,
                    0,
                    self.size.0,
                    self.size.1,
                );
            }
        }
        if let Some(rect) = now {
            let _ = conn.copy_area(
                self.bright,
                self.window,
                self.gc,
                rect.x,
                rect.y,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            );
            let _ = conn.poly_rectangle(self.window, self.gc, &[rect]);
        }
        let _ = conn.flush();
    }

    /// A rectangle in the window, in the still's pixels.
    fn in_still(&self, rect: Rectangle) -> Selection {
        let sx = f64::from(self.still.width) / f64::from(self.size.0.max(1));
        let sy = f64::from(self.still.height) / f64::from(self.size.1.max(1));
        let x = (f64::from(rect.x.max(0)) * sx) as u32;
        let y = (f64::from(rect.y.max(0)) * sy) as u32;
        let width = (f64::from(rect.width) * sx).ceil() as u32;
        let height = (f64::from(rect.height) * sy).ceil() as u32;
        (x, y, width, height)
    }

    fn close(mut self) {
        let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.conn.destroy_window(self.window);
        self.free_pixmaps();
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.flush();
    }
}

/// Names the window and, when a window manager is placing it, asks for it
/// full-screen and on top.
///
/// The class is its own, so a compositor rule can tell it apart from the
/// bubble — to turn off an opacity or animation rule that suits ordinary
/// windows and not a frozen picture of the screen.
fn describe(conn: &RustConnection, window: u32, override_redirect: bool) -> Result<(), String> {
    let atom = |name: &str| -> Result<u32, String> {
        Ok(conn
            .intern_atom(false, name.as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom)
    };
    let title = b"bubbleTranslate \xe2\x80\x94 read the screen";
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"bubbleTranslate - read the screen",
    )
    .map_err(|e| e.to_string())?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        atom("_NET_WM_NAME")?,
        atom("UTF8_STRING")?,
        title,
    )
    .map_err(|e| e.to_string())?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"bubbleTranslate-overlay\0bubbleTranslate-overlay\0",
    )
    .map_err(|e| e.to_string())?;
    if !override_redirect {
        let state = [atom("_NET_WM_STATE_FULLSCREEN")?, atom("_NET_WM_STATE_ABOVE")?];
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atom("_NET_WM_STATE")?,
            AtomEnum::ATOM,
            &state,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Tells Hyprland, once per run, how to show the overlay.
///
/// Hyprland does not act on an XWayland window asking to be full-screen —
/// neither the property set before mapping nor the request sent after — and
/// tiles it beside whatever was open, which is a picture of the screen shrunk
/// into half of it. A window rule is the one thing it does read at map time.
/// Floating as well as full-screen: a full-screen window that is tiled still
/// takes a tile, and the windows behind it are squeezed aside while the
/// overlay is up and slide back when it closes.
///
/// The same rule turns off the opening animation and any opacity the user
/// gives ordinary windows: a frozen picture of the screen that fades in, or
/// that the live screen shows through, is not the screen any more.
///
/// Added at runtime rather than asked of the user, and gone with the next
/// config reload; after one, the next run adds it again. Best effort — a
/// Hyprland that refuses it tiles the overlay, and the drag still works.
fn hyprland_rule() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    ONCE.call_once(|| {
        let run = |args: &[&str]| {
            let mut command = std::process::Command::new("hyprctl");
            command.args(args);
            super::timed_output(command, super::IPC_BUDGET)
                .is_ok_and(|out| out.status.success() && !out.stdout.starts_with(b"Error"))
        };
        // 0.56 and later configure in Lua; before that, in rule strings.
        let lua = "hl.window_rule({ match = { class = '^bubbleTranslate-overlay$' }, \
                   float = true, fullscreen = true, no_anim = true, \
                   tag = '-default-opacity', \
                   opacity = '1 1' })";
        let added = run(&["eval", lua])
            || run(&[
                "keyword",
                "windowrulev2",
                "fullscreen,class:^(bubbleTranslate-overlay)$",
            ])
                && run(&[
                    "keyword",
                    "windowrulev2",
                    "float,class:^(bubbleTranslate-overlay)$",
                ]);
        crate::trace!("overlay   hyprland window rule added: {added}");
    });
}

/// The crosshair pointer, from the cursor font every X server carries.
fn crosshair(conn: &RustConnection) -> Option<u32> {
    let font = conn.generate_id().ok()?;
    conn.open_font(font, b"cursor").ok()?;
    let cursor = conn.generate_id().ok()?;
    conn.create_glyph_cursor(
        cursor,
        font,
        font,
        XC_CROSSHAIR,
        XC_CROSSHAIR + 1,
        0xffff,
        0xffff,
        0xffff,
        0,
        0,
        0,
    )
    .ok()?;
    let _ = conn.close_font(font);
    Some(cursor)
}

/// The rectangle the two corners describe, whichever way the drag went.
fn between(a: (i16, i16), b: (i16, i16)) -> Rectangle {
    Rectangle {
        x: a.0.min(b.0),
        y: a.1.min(b.1),
        width: (i32::from(a.0) - i32::from(b.0)).unsigned_abs() as u16,
        height: (i32::from(a.1) - i32::from(b.1)).unsigned_abs() as u16,
    }
}

fn grow(rect: Rectangle, by: i16) -> Rectangle {
    Rectangle {
        x: rect.x.saturating_sub(by),
        y: rect.y.saturating_sub(by),
        width: rect.width.saturating_add(2 * by as u16),
        height: rect.height.saturating_add(2 * by as u16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_backwards_is_the_same_rectangle() {
        let parts = |r: Rectangle| (r.x, r.y, r.width, r.height);
        let forward = parts(between((10, 20), (110, 70)));
        let backward = parts(between((110, 70), (10, 20)));
        assert_eq!(forward, backward);
        assert_eq!(forward, (10, 20, 100, 50));
    }
}
