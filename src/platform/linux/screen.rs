//! Taking a still picture of the screen, before anything is drawn over it.
//!
//! The picture is taken first and the rectangle is chosen on it afterwards,
//! the other way round from Windows. Here the overlay the rectangle is drawn
//! on is an ordinary window, and a compositor will not let an ordinary window
//! be see-through to the pixels beneath without also blending them — so the
//! overlay shows a frozen copy of the screen instead, and what is read is
//! exactly the copy the user was looking at while they drew.
//!
//! **X11** hands the root window's pixels to any client that asks.
//!
//! **Wayland** does not, on purpose; there is no core protocol for reading
//! another client's pixels. `wlr-screencopy` is the extension that exists for
//! screenshot tools, and the wlroots compositors — Hyprland, sway, river,
//! Wayfire — implement it. GNOME and KDE do not: there a screenshot goes
//! through the desktop's own portal instead, see [`super::portal`].

use std::fs::File;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::fs::FileExt;

use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

use super::ocr::Frame;

/// A picture of one screen, and where that screen is.
pub struct Shot {
    pub frame: Frame,
    /// The screen's top-left corner, in the space pointer positions are
    /// reported in — so a rectangle on the picture can be turned into a place
    /// to put the bubble.
    pub origin: (f64, f64),
    /// The screen's size in that same space. On a scaled display it is
    /// smaller than the picture, which is taken in real pixels.
    pub size: (f64, f64),
    pub placement: Placement,
}

impl Shot {
    /// How many of the picture's pixels make one unit of the pointer's space.
    pub fn scale(&self) -> f64 {
        if self.size.0 > 0.0 {
            f64::from(self.frame.width) / self.size.0
        } else {
            1.0
        }
    }
}

/// How the overlay that shows the picture has to be put on the screen.
#[derive(Debug, Clone, Copy)]
pub enum Placement {
    /// Ask the window manager for a full-screen window, and let it choose
    /// which screen. Under XWayland that is the only way to cover a screen:
    /// X11 coordinates there do not reliably line up with the compositor's.
    Fullscreen,
    /// Cover exactly this rectangle of the X11 root window, placed by the
    /// overlay itself. A real X11 session, where coordinates mean what they
    /// say and there may be no window manager to ask.
    Cover {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
    },
    /// Like [`Placement::Fullscreen`], but the picture is the whole desktop
    /// rather than one screen — the portal takes every monitor at once — so
    /// the overlay shows the part of it under wherever the window manager
    /// put the window. The picture spans the X11 root window, which is how
    /// that part is found.
    Desktop,
    /// No overlay at all: the desktop's own screenshot interface already had
    /// the user choose, and the picture is the region.
    Chosen,
}

/// Takes the picture, of the screen the pointer is on.
///
/// The error is a sentence for the trace, not for the user: a desktop that
/// cannot be read is a platform limit, and the key simply does nothing there.
pub fn grab() -> Result<Shot, String> {
    // For trying the GNOME and KDE route on a desktop that has a better one.
    if std::env::var_os("BUBBLETRANSLATE_SCREENSHOT").is_some_and(|v| v == "portal") {
        return portal_grab();
    }
    if super::on_wayland() {
        wayland_grab().or_else(|err| {
            crate::trace!("screen    {err}; asking the desktop's portal instead");
            portal_grab()
        })
    } else if std::env::var_os("DISPLAY").is_some() {
        x11_grab()
    } else {
        Err("no display server".into())
    }
}

// --- X11 --------------------------------------------------------------------

fn x11_grab() -> Result<Shot, String> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder};

    let (conn, screen_num) =
        x11rb::connect(None).map_err(|err| format!("could not reach the X server: {err}"))?;
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    if !super::overlay::pixels_are_bgrx(setup, screen.root_depth)
        || setup.image_byte_order != ImageOrder::LSB_FIRST
    {
        return Err(format!(
            "the X server stores depth-{} pixels in a layout this cannot read",
            screen.root_depth
        ));
    }

    let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, width, height, !0)
        .map_err(|err| format!("could not ask for the screen: {err}"))?
        .reply()
        .map_err(|err| format!("the X server would not hand over the screen: {err}"))?;

    let expected = usize::from(width) * usize::from(height) * 4;
    if reply.data.len() < expected {
        return Err("the X server sent a shorter picture than it described".into());
    }
    let mut bgrx = reply.data;
    bgrx.truncate(expected);

    Ok(Shot {
        frame: Frame {
            width: u32::from(width),
            height: u32::from(height),
            bgrx,
        },
        origin: (0.0, 0.0),
        size: (f64::from(width), f64::from(height)),
        placement: Placement::Cover {
            x: 0,
            y: 0,
            width,
            height,
        },
    })
}

// --- the portal -------------------------------------------------------------

fn portal_grab() -> Result<Shot, String> {
    use x11rb::connection::Connection as _;

    // The overlay is an X11 window, so the X11 root is the space the picture
    // is laid over: under XWayland it spans every monitor, as the portal's
    // picture does.
    let root = x11rb::connect(None).ok().map(|(conn, n)| {
        let screen = &conn.setup().roots[n];
        (
            f64::from(screen.width_in_pixels),
            f64::from(screen.height_in_pixels),
        )
    });

    match super::portal::grab()? {
        super::portal::Picture::Desktop(frame) => {
            let size = root.unwrap_or((f64::from(frame.width), f64::from(frame.height)));
            Ok(Shot {
                frame,
                origin: (0.0, 0.0),
                size,
                placement: Placement::Desktop,
            })
        }
        super::portal::Picture::Chosen(frame) => {
            let size = (f64::from(frame.width), f64::from(frame.height));
            Ok(Shot {
                frame,
                origin: (0.0, 0.0),
                size,
                placement: Placement::Chosen,
            })
        }
    }
}

// --- Wayland ----------------------------------------------------------------

/// One screen as the compositor describes it.
#[derive(Default)]
struct Output {
    output: Option<wl_output::WlOutput>,
    /// Where it sits and how big it is, in logical coordinates — the unit
    /// pointer positions are reported in.
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

/// The buffer the compositor asked to be given.
#[derive(Clone, Copy)]
struct BufferSpec {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Default)]
struct State {
    shm: Option<wl_shm::WlShm>,
    manager: Option<(zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, u32)>,
    xdg: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: Vec<Output>,
    spec: Option<BufferSpec>,
    buffer_done: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

fn wayland_grab() -> Result<Shot, String> {
    let conn = Connection::connect_to_env()
        .map_err(|err| format!("could not connect to the Wayland compositor: {err}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|err| format!("the Wayland compositor did not answer: {err}"))?;

    let Some((manager, version)) = state.manager.clone() else {
        return Err("this compositor does not let applications read the screen \
                    (no wlr-screencopy)"
            .into());
    };
    let Some(shm) = state.shm.clone() else {
        return Err("this compositor offers no shared memory".into());
    };

    if let Some(xdg) = state.xdg.clone() {
        for (index, output) in state.outputs.iter().enumerate() {
            if let Some(wl) = &output.output {
                xdg.get_xdg_output(wl, &qh, index);
            }
        }
        queue
            .roundtrip(&mut state)
            .map_err(|err| format!("the Wayland compositor did not answer: {err}"))?;
    }

    let index = pick_output(&state.outputs, super::cursor::position())
        .ok_or("this compositor reported no screens")?;
    let wl = state.outputs[index]
        .output
        .clone()
        .ok_or("the screen went away")?;

    // The pointer is left out of the picture: it would be frozen into the
    // backdrop at the spot the key was pressed, and a second arrow that does
    // not move is the first thing anyone would try to click.
    let frame = manager.capture_output(0, &wl, &qh, ());

    // Version 3 lists every buffer type it could fill and then says it is
    // done; before that, the one shared-memory description is all there is.
    while !(state.failed || state.buffer_done || (version < 3 && state.spec.is_some())) {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| format!("the Wayland connection ended: {err}"))?;
    }
    let spec = match (state.failed, state.spec) {
        (false, Some(spec)) => spec,
        _ => return Err("the compositor would not describe the screen's buffer".into()),
    };

    let bytes = u64::from(spec.stride) * u64::from(spec.height);
    let file = memfd(bytes).map_err(|err| format!("could not make a buffer: {err}"))?;
    let pool = shm.create_pool(file.as_fd(), bytes as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        spec.width as i32,
        spec.height as i32,
        spec.stride as i32,
        spec.format,
        &qh,
        (),
    );
    frame.copy(&buffer);

    while !(state.ready || state.failed) {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| format!("the Wayland connection ended: {err}"))?;
    }
    frame.destroy();
    buffer.destroy();
    pool.destroy();
    if state.failed {
        return Err("the compositor could not copy the screen".into());
    }

    let mut raw = vec![0u8; bytes as usize];
    file.read_exact_at(&mut raw, 0)
        .map_err(|err| format!("could not read the copied screen: {err}"))?;
    let bgrx = to_bgrx(&raw, spec, state.y_invert)?;

    let output = &state.outputs[index];
    let origin = output.position.unwrap_or((0, 0));
    let size = output
        .size
        .unwrap_or((spec.width as i32, spec.height as i32));

    Ok(Shot {
        frame: Frame {
            width: spec.width,
            height: spec.height,
            bgrx,
        },
        origin: (f64::from(origin.0), f64::from(origin.1)),
        size: (f64::from(size.0), f64::from(size.1)),
        placement: Placement::Fullscreen,
    })
}

/// The screen the pointer is on, or the first one when nobody will say where
/// the pointer is.
fn pick_output(outputs: &[Output], pointer: Option<(f64, f64)>) -> Option<usize> {
    if outputs.is_empty() {
        return None;
    }
    let under = pointer.and_then(|(px, py)| {
        outputs.iter().position(|output| {
            let (Some((x, y)), Some((w, h))) = (output.position, output.size) else {
                return false;
            };
            px >= f64::from(x)
                && px < f64::from(x + w)
                && py >= f64::from(y)
                && py < f64::from(y + h)
        })
    });
    Some(under.unwrap_or(0))
}

/// Shared memory the compositor can write into and we can read back from,
/// which is all a Wayland buffer is.
fn memfd(len: u64) -> std::io::Result<File> {
    // SAFETY: the name is a valid C string; the call has no other inputs.
    let fd = unsafe { libc::memfd_create(c"bubbleTranslate-screen".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the descriptor was just created and nothing else owns it.
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(len)?;
    Ok(file)
}

/// Rewrites the compositor's buffer as top-down BGRX, dropping each row's
/// padding.
fn to_bgrx(raw: &[u8], spec: BufferSpec, y_invert: bool) -> Result<Vec<u8>, String> {
    // Little-endian 32-bit words: ARGB/XRGB is B, G, R, A in memory, which is
    // already the order wanted; ABGR/XBGR has red and blue the other way.
    let swap = match spec.format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => false,
        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => true,
        other => {
            return Err(format!(
                "the screen is in a pixel format this cannot read: {other:?}"
            ));
        }
    };
    let row_bytes = (spec.width * 4) as usize;
    let mut out = Vec::with_capacity(row_bytes * spec.height as usize);
    for row in 0..spec.height {
        let from = if y_invert { spec.height - 1 - row } else { row };
        let start = (from * spec.stride) as usize;
        let line = raw
            .get(start..start + row_bytes)
            .ok_or("the copied screen is shorter than described")?;
        if swap {
            for pixel in line.as_chunks::<4>().0 {
                out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        } else {
            out.extend_from_slice(line);
        }
    }
    Ok(out)
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
            "wl_output" => {
                let index = state.outputs.len();
                state.outputs.push(Output {
                    output: Some(registry.bind(name, version.min(2), qh, index)),
                    ..Output::default()
                });
            }
            "zxdg_output_manager_v1" => {
                state.xdg = Some(registry.bind(name, version.min(2), qh, ()))
            }
            "zwlr_screencopy_manager_v1" => {
                let version = version.min(3);
                state.manager = Some((registry.bind(name, version, qh, ()), version));
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.size = Some((width, height))
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::{Event, Flags};
        match event {
            Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                state.spec = Some(BufferSpec {
                    format,
                    width,
                    height,
                    stride,
                })
            }
            Event::BufferDone => state.buffer_done = true,
            Event::Flags {
                flags: WEnum::Value(flags),
            } => state.y_invert = flags.contains(Flags::YInvert),
            Event::Ready { .. } => state.ready = true,
            Event::Failed => state.failed = true,
            _ => {}
        }
    }
}

/// The objects that send nothing this needs to hear.
macro_rules! quiet {
    ($($interface:ty => $data:ty),* $(,)?) => {$(
        impl Dispatch<$interface, $data> for State {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

quiet! {
    wl_shm::WlShm => (),
    wl_shm_pool::WlShmPool => (),
    wl_buffer::WlBuffer => (),
    wl_output::WlOutput => usize,
    zxdg_output_manager_v1::ZxdgOutputManagerV1 => (),
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 => (),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(x: i32, y: i32, w: i32, h: i32) -> Output {
        Output {
            output: None,
            position: Some((x, y)),
            size: Some((w, h)),
        }
    }

    #[test]
    fn the_screen_under_the_pointer_is_the_one_read() {
        let outputs = [output(0, 0, 1280, 800), output(1280, 0, 1920, 1080)];
        assert_eq!(pick_output(&outputs, Some((1500.0, 300.0))), Some(1));
        assert_eq!(pick_output(&outputs, Some((10.0, 10.0))), Some(0));
        // Nowhere, or not known: the first screen rather than none.
        assert_eq!(pick_output(&outputs, None), Some(0));
        assert_eq!(pick_output(&outputs, Some((-50.0, -50.0))), Some(0));
        assert_eq!(pick_output(&[], None), None);
    }

    #[test]
    fn red_and_blue_trade_places_for_abgr_and_padding_is_dropped() {
        let spec = BufferSpec {
            format: wl_shm::Format::Xbgr8888,
            width: 1,
            height: 2,
            stride: 8,
        };
        let raw = [1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 0, 9, 9, 9, 9];
        assert_eq!(
            to_bgrx(&raw, spec, false).unwrap(),
            [3, 2, 1, 0, 6, 5, 4, 0]
        );
        assert_eq!(to_bgrx(&raw, spec, true).unwrap(), [6, 5, 4, 0, 3, 2, 1, 0]);
    }
}
