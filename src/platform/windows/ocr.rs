//! Reading text off the screen when there is no selection to read.
//!
//! Everything else in [`super::capture`] asks an application what it has
//! selected. Some of them cannot answer at all: an image, a video frame, a
//! game, a remote desktop, a PDF that draws its own glyphs. There is text on
//! the screen and no way to ask for it, so it is read the way a person reads
//! it — off the pixels.
//!
//! **No permission is involved, and that is the whole difference from macOS.**
//! Reading the screen there needs Screen Recording consent, granted by hand in
//! System Settings, and until it is granted the capture comes back blank —
//! which is why the macOS build that does this ships separately, so nobody is
//! asked for a permission they did not want. Windows attaches nothing to
//! `BitBlt` from the screen DC: no prompt, no consent, no setting, and nothing
//! for [`super::capture::readiness`] to warn about. So this is an ordinary
//! part of the ordinary build.
//!
//! **Why GDI rather than `Windows.Graphics.Capture`.** The modern capture API
//! wants a `DispatcherQueue` on the calling thread, hands frames back through
//! a callback, and in some Windows builds draws a yellow border around what it
//! records. For one still rectangle, none of that buys anything. `BitBlt` into
//! a DIB section is synchronous, gives the pixels in exactly the layout the
//! OCR engine wants, and has worked unchanged since Windows 3.
//!
//! **The engine is the one Windows ships.** `Windows.Media.Ocr` is offline,
//! free, and already on the machine — no model to download and nothing sent
//! anywhere, which matters for a translator people point at whatever is on
//! their screen. What it cannot do is recognise a language whose pack is not
//! installed: see [`languages`].
//!
//! Nothing calls this yet: what picks the region — a hotkey, a drag over a
//! dimmed screen, an item in the tray menu — is the half that has to match
//! what the macOS build already does, and until the two agree this module is
//! the engine with no ignition. Hence the allow; it comes off with the first
//! caller.
#![allow(dead_code)]

use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};

/// A rectangle of the virtual desktop to read, in **physical pixels** with a
/// top-left origin.
///
/// Physical rather than the toolkit's points, because that is what `BitBlt`
/// speaks and because the whole point is to read the pixels that are actually
/// lit. Whatever picks the region is responsible for the conversion, in the
/// opposite direction to [`super::to_points`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Region {
    /// Whether there is anything here to read.
    ///
    /// A drag that ended where it started is the usual way this happens, and
    /// it is a cancel rather than an error — a zero-width `BitBlt` fails, and
    /// a one-pixel one succeeds and wastes an OCR pass.
    fn is_usable(&self) -> bool {
        self.width >= MIN_SIDE && self.height >= MIN_SIDE
    }
}

/// The smallest side worth reading, in pixels. Below this there is no glyph in
/// the region, only the end of a stray drag.
const MIN_SIDE: i32 = 8;

/// The largest region that will be read, as a pixel count.
///
/// The whole of a 4K desktop is about 8 megapixels and takes an appreciable
/// moment to recognise; several monitors of it is not a translation request.
/// A region past this is read anyway — refusing would be worse — but it is
/// worth knowing the ceiling exists when the bubble seems slow.
const MAX_PIXELS: i64 = 16_000_000;

/// Reads whatever text is in `region`.
///
/// `None` when the region is too small to hold a glyph, when the screen could
/// not be read, or when no recognition language is installed. An empty string
/// is a different answer and a real one: the pixels were read and held no
/// text.
pub fn recognize(region: Region) -> Option<String> {
    if !region.is_usable() {
        crate::trace!(
            "ocr       region {}x{} too small to read",
            region.width,
            region.height
        );
        return None;
    }
    if i64::from(region.width) * i64::from(region.height) > MAX_PIXELS {
        crate::trace!(
            "ocr       region {}x{} is larger than the {MAX_PIXELS}px ceiling; reading it anyway",
            region.width,
            region.height
        );
    }

    let pixels = grab(region)?;
    let bitmap = to_bitmap(&pixels, region.width, region.height).ok()?;

    let engine = match OcrEngine::TryCreateFromUserProfileLanguages() {
        Ok(engine) => engine,
        Err(error) => {
            crate::trace!("ocr       no engine for the user's languages ({error})");
            return None;
        }
    };

    // `join` blocks this thread until the recognition finishes. That is what
    // is wanted: this already runs off the UI thread, on the same worker that
    // a clipboard capture would have used, and the bubble is waiting on it.
    let text = engine
        .RecognizeAsync(&bitmap)
        .and_then(|pending| pending.join())
        .and_then(|result| result.Text())
        .ok()?
        .to_string_lossy();

    crate::trace!(
        "ocr       read {} chars from {}x{}",
        text.chars().count(),
        region.width,
        region.height
    );
    Some(text)
}

/// Copies `region` off the screen as top-down BGRA.
///
/// BGRA because that is `BitmapPixelFormat::Bgra8`, which is what the OCR
/// engine takes without a conversion, and it is also what a 32-bit DIB section
/// already is — so the bytes are handed over exactly as `BitBlt` left them.
fn grab(region: Region) -> Option<Vec<u8>> {
    // SAFETY: every handle below is released on every path out, including the
    // early returns, which is why the failures are collected at the end rather
    // than returned where they happen.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            crate::trace!("ocr       could not get the screen DC");
            return None;
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.is_invalid() {
            ReleaseDC(None, screen);
            crate::trace!("ocr       could not make a memory DC");
            return None;
        }

        // A negative height asks for a top-down bitmap: the first row in the
        // buffer is the top row of the screen. Bottom-up is the DIB default
        // and would hand the OCR engine an upside-down image.
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: region.width,
                biHeight: -region.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0);

        let pixels = match dib {
            Ok(dib) if !bits.is_null() => {
                let previous = SelectObject(memory, HGDIOBJ(dib.0));
                // `CAPTUREBLT` includes layered windows — tooltips, overlays,
                // and anything drawn with transparency. Without it they are
                // holes in the capture, and a tooltip is exactly the kind of
                // text somebody points this at.
                let copied = BitBlt(
                    memory,
                    0,
                    0,
                    region.width,
                    region.height,
                    Some(screen),
                    region.x,
                    region.y,
                    SRCCOPY | CAPTUREBLT,
                )
                .is_ok();

                let pixels = copied.then(|| {
                    let len = region.width as usize * region.height as usize * 4;
                    std::slice::from_raw_parts(bits as *const u8, len).to_vec()
                });

                SelectObject(memory, previous);
                let _ = DeleteObject(HGDIOBJ(dib.0));
                pixels
            }
            _ => None,
        };

        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);

        if pixels.is_none() {
            crate::trace!("ocr       the screen copy failed");
        }
        pixels
    }
}

/// Wraps raw BGRA bytes as the bitmap the OCR engine takes.
///
/// Through a `DataWriter` rather than a file: the obvious route in every
/// example is to save a PNG and decode it back, which writes the user's screen
/// to disk for no reason at all. This keeps the pixels in memory from the
/// screen to the engine.
fn to_bitmap(bgra: &[u8], width: i32, height: i32) -> windows::core::Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(bgra)?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
}

/// The BCP-47 tags the machine can actually recognise, such as `en-US`.
///
/// Worth surfacing, because this is the one place Windows is meaningfully
/// poorer than macOS: recognition languages arrive as separate
/// `Language.OCR~~~<tag>` capabilities, installed alongside a display
/// language, and a fresh Windows often has only the one it was installed in.
/// Vision, by contrast, brings a broad set with the system.
///
/// The practical shape of that: the engine has to read the language *being
/// translated from*, so somebody reading English on a Turkish machine is
/// already served. Somebody pointing this at Turkish text on a machine with
/// only `en-US` is not, and gets confident nonsense rather than an error —
/// which is the reason to be able to show this list rather than let it fail
/// quietly.
pub fn languages() -> Vec<String> {
    OcrEngine::AvailableRecognizerLanguages()
        .into_iter()
        .flatten()
        .filter_map(|language| Some(language.LanguageTag().ok()?.to_string_lossy()))
        .collect()
}

/// Whether reading the screen will work at all on this machine.
pub fn available() -> bool {
    OcrEngine::TryCreateFromUserProfileLanguages().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_with_no_area_is_not_worth_reading() {
        let empty = Region {
            x: 100,
            y: 100,
            width: 0,
            height: 0,
        };
        assert!(!empty.is_usable());
        assert_eq!(recognize(empty), None);
    }

    #[test]
    fn a_stray_drag_is_not_worth_reading() {
        let stray = Region {
            x: 0,
            y: 0,
            width: MIN_SIDE - 1,
            height: 200,
        };
        assert!(!stray.is_usable());
    }

    #[test]
    fn an_ordinary_region_is_worth_reading() {
        assert!(
            Region {
                x: 0,
                y: 0,
                width: 400,
                height: 120,
            }
            .is_usable()
        );
    }

    /// Not an assertion about *which* languages are installed — that is a
    /// property of the machine, and CI's is not the developer's. Only that
    /// asking is safe on a machine with none.
    #[test]
    fn asking_for_the_languages_does_not_panic() {
        let _ = languages();
        let _ = available();
    }

    /// Reads the top-left corner of the real screen, and is ignored by
    /// default because of it: it needs a desktop with something on it, which
    /// a CI runner does not have. Run it by hand on a machine showing text:
    ///
    /// ```text
    /// cargo test --target x86_64-pc-windows-msvc ocr -- --ignored --nocapture
    /// ```
    ///
    /// It asserts only that the pipeline completes and says what it read, so
    /// the eye can judge the rest. Asserting on the words would be asserting
    /// on whatever happened to be on screen.
    #[test]
    #[ignore = "reads the real screen; needs a desktop"]
    fn reads_the_real_screen() {
        let region = Region {
            x: 0,
            y: 0,
            width: 1200,
            height: 800,
        };

        println!("recognition languages: {:?}", languages());
        assert!(available(), "no OCR language is installed on this machine");

        let text = recognize(region).expect("the screen should be readable");
        println!("--- {} chars read ---\n{text}", text.chars().count());
    }
}
