//! Translating the whole screen in place on macOS: Cmd+Shift+F.
//!
//! The three steps every system does its own way, done here with what macOS
//! ships. The screen is photographed with Core Graphics, read with the same
//! on-device Vision recogniser the rectangle uses — but with the box around
//! every word kept — and the finished still is shown in a borderless window
//! that covers the screen until a key or a click puts it away. Everything in
//! between — grouping the words into paragraphs, matching the lettering and
//! painting the translations in — is [`crate::page`], the part every system
//! shares.
//!
//! The window has to be made on the main thread, where AppKit insists every
//! window is made; the worker that photographed and painted the screen hands
//! that last step over through the main dispatch queue and waits for the key
//! or click that the event tap reports back.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::mpsc::Sender;

use crate::page::Block;

/// A picture of the screen: four bytes a pixel, blue first, top row first —
/// the layout [`crate::page`] reads and writes.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgrx: Vec<u8>,
}

/// A photographed screen, ready to be read and then painted over.
pub struct PageShot {
    frame: Frame,
    /// How many of the picture's pixels make one point — 2 on a Retina
    /// display. The note along the bottom is sized in points, so it needs it.
    scale: f64,
}

/// Registers the callback for Cmd+Shift+F.
pub fn on_screen_page_request(ask: impl Fn() + Send + 'static) {
    super::monitor::on_page_request(ask);
}

/// Photographs the screen the way the rectangle reader does: only once the
/// permission is in hand, and never if there is no recogniser to read it with.
///
/// `None` for each way it comes to nothing — the permission withheld, the
/// capture refused — none of which is worth a bubble.
pub fn capture_page() -> Option<PageShot> {
    crate::trace!("page      asked to translate the screen");
    if !super::granted() {
        // The same as the rectangle: asking opens System Settings and puts
        // the app in the list, and macOS only hands the grant to a fresh
        // launch, so this run still comes back empty.
        crate::trace!("page      no screen-recording permission; asking for it");
        core_graphics::access::ScreenCaptureAccess.request();
        super::shell::open_url(super::SCREEN_SETTINGS);
        return None;
    }
    if !super::ocr::available() {
        crate::trace!("ocr       no recognition language installed");
        return None;
    }

    let display = core_graphics::display::CGDisplay::main();
    let image = display.image()?;
    let frame = frame_from_image(&image)?;
    let bounds = display.bounds();
    let scale = if bounds.size.width > 0.0 {
        f64::from(frame.width) / bounds.size.width
    } else {
        1.0
    };
    crate::trace!(
        "page      captured {}x{} at scale {scale}",
        frame.width,
        frame.height
    );
    Some(PageShot { frame, scale })
}

/// Reads the photographed screen into the paragraphs that get translated.
///
/// The twin of the Linux reader: the words come back with their boxes, the
/// lines they make are tightened to their ink and dropped where the matching
/// face would not land on the letters, and what is left is grouped into
/// paragraphs.
/// The photograph, ready for Claude to read; see [`crate::picture`].
pub fn page_picture(shot: &PageShot) -> Option<Vec<u8>> {
    crate::picture::encode(shot.frame.width, shot.frame.height, &shot.frame.bgrx)
}

pub fn read_page(shot: &PageShot) -> Vec<Block> {
    let frame = &shot.frame;
    let Some(png) = encode_png(frame) else {
        crate::trace!("page      could not encode the screen to read it");
        return Vec::new();
    };
    let source_lang = crate::config::Config::load().source_lang;
    let word_lines = super::ocr::recognize_words(&png, &source_lang, frame.width, frame.height);
    let lines = crate::page::lines_from_words(word_lines);
    let pixels = crate::page::Pixels {
        width: frame.width,
        height: frame.height,
        bgrx: &frame.bgrx,
    };
    if crate::trace::enabled() {
        for line in &lines {
            crate::trace!("page      read {:?} {:?}", line.rect, line.text);
        }
    }
    let fonts = crate::page::fonts();
    // A line the reader handed back as two halves is put back together
    // first; see [`crate::page::join_split_lines`].
    let tightened = crate::page::join_split_lines(lines)
        .into_iter()
        .map(|line| pixels.tighten(line))
        .collect();
    // Vision occasionally reports the same line twice; dropping the overlaps
    // before anything is measured keeps a word from being painted over itself.
    let lines = crate::page::without_duplicates(tightened)
        .into_iter()
        .filter(|line| pixels.replaceable(fonts, line))
        .collect();
    crate::page::paragraphs(lines, |rect| pixels.column_edge(rect))
}

/// Paints the translations onto the photographed screen and shows it in place
/// of the screen, until a key or a click puts it away.
pub fn show_page(shot: PageShot, translated: &[(Block, String)], note: Option<&str>) {
    let PageShot { mut frame, scale } = shot;
    let fonts = crate::page::fonts();
    let mut canvas = crate::page::Canvas {
        width: frame.width,
        height: frame.height,
        bgrx: &mut frame.bgrx,
    };
    let started = std::time::Instant::now();
    canvas.draw_page(fonts, translated);
    crate::trace!("page      painted in {:?}", started.elapsed());
    if let Some(note) = note {
        canvas.note(fonts, note, scale as f32);
    }
    overlay::show(frame);
}

/// Told by the event tap of a key or a click, so it can put a translated
/// screen away. A lock and a `None` when none is up, which costs nothing.
pub fn note_input() {
    overlay::note_input();
}

/// Copies a Core Graphics screenshot into a tight blue-first buffer.
///
/// The capture is 32-bit, blue-green-red-alpha in memory, with a row that may
/// be padded past the picture's width; this drops the padding and the alpha so
/// what is left is exactly what [`crate::page`] reads.
fn frame_from_image(image: &core_graphics::image::CGImage) -> Option<Frame> {
    let width = image.width() as u32;
    let height = image.height() as u32;
    // A screen capture is 32-bit, one byte a channel; anything else — a
    // 64-bit wide-gamut capture, say — would be read as nonsense here, so the
    // screen is left untranslated rather than painted with garbage.
    let bytes_per_pixel = image.bits_per_pixel() / 8;
    if bytes_per_pixel != 4 {
        crate::trace!("page      unexpected capture format: {bytes_per_pixel} bytes a pixel");
        return None;
    }
    let stride = image.bytes_per_row();
    let data = image.data();
    let bytes = data.bytes();
    if bytes.len() < stride * height as usize {
        return None;
    }
    let row_pixels = width as usize;
    let mut bgrx = vec![0u8; row_pixels * height as usize * 4];
    for y in 0..height as usize {
        let src = &bytes[y * stride..];
        let dst = &mut bgrx[y * row_pixels * 4..];
        for x in 0..row_pixels {
            let s = x * bytes_per_pixel;
            let d = x * 4;
            dst[d] = src[s]; // blue
            dst[d + 1] = src[s + 1]; // green
            dst[d + 2] = src[s + 2]; // red
            dst[d + 3] = 255; // the fourth byte is ignored, but kept full
        }
    }
    Some(Frame {
        width,
        height,
        bgrx,
    })
}

/// Encodes a frame as a PNG, the one thing both the recogniser and the overlay
/// take. Blue-first in memory becomes red-first for the PNG.
fn encode_png(frame: &Frame) -> Option<Vec<u8>> {
    let mut rgba = vec![0u8; frame.bgrx.len()];
    for (dst, src) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(frame.bgrx.as_chunks::<4>().0) {
        dst[0] = src[2]; // red
        dst[1] = src[1]; // green
        dst[2] = src[0]; // blue
        dst[3] = 255;
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

/// The borderless window that shows a translated screen, and the way the tap
/// closes it.
mod overlay {
    use super::{Frame, Mutex, RefCell, Sender, encode_png};

    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSImage, NSImageScaling, NSImageView, NSScreen, NSScreenSaverWindowLevel,
        NSWindow, NSWindowStyleMask,
    };
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::NSData;

    /// The sender the waiting worker is parked on, set while a translated
    /// screen is up and taken by the first key or click.
    static DISMISS: Mutex<Option<Sender<()>>> = Mutex::new(None);

    thread_local! {
        /// The window, held on the main thread that made it — the only thread
        /// that may touch it — so the dismissal can order it away again.
        static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    }

    /// Shows `frame` full-screen and blocks until a key or a click.
    ///
    /// The long stop is a safety net, not a timeout anyone should reach: a
    /// missed event must never wedge the worker — and with it every later
    /// screen translation — for the rest of the session.
    pub fn show(frame: Frame) {
        let Some(png) = encode_png(&frame) else {
            crate::trace!("page      could not encode the screen to show it");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        *DISMISS.lock().unwrap() = Some(tx);

        dispatch2::DispatchQueue::main().exec_async(move || present(&png));

        let _ = rx.recv_timeout(std::time::Duration::from_secs(300));
        *DISMISS.lock().unwrap() = None;
        dispatch2::DispatchQueue::main().exec_async(dismiss_window);
    }

    /// Wakes the worker, once, on the first key or click the tap sees while a
    /// screen is up.
    pub fn note_input() {
        if let Some(tx) = DISMISS.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    /// Makes the window and brings it to the front. On the main thread.
    fn present(png: &[u8]) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(screen) = NSScreen::mainScreen(mtm) else {
            return;
        };
        let frame_rect = screen.frame();

        let data = NSData::with_bytes(png);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            crate::trace!("page      the screen picture would not load");
            return;
        };

        let view = NSImageView::imageViewWithImage(&image, mtm);
        // The picture is in real pixels and the window is in points; letting
        // the view scale it to the window draws it life-size, Retina included.
        view.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        let content = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: frame_rect.size.width,
                height: frame_rect.size.height,
            },
        };
        view.setFrame(content);

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame_rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setLevel(NSScreenSaverWindowLevel);
        window.setOpaque(true);
        window.setContentView(Some(&view));
        // Shown without being made key: taking focus would pull it from the
        // application being translated, and the tap hears the dismissing key
        // wherever it lands.
        window.orderFrontRegardless();

        WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
    }

    /// Orders the window away. On the main thread.
    fn dismiss_window() {
        WINDOW.with(|slot| {
            if let Some(window) = slot.borrow_mut().take() {
                window.orderOut(None);
            }
        });
    }
}
