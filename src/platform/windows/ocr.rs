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

use std::sync::Mutex;

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::core::HSTRING;

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

/// A picture in memory: `width * height * 4` bytes, B, G, R and one unused,
/// top row first.
///
/// That is the layout `BitBlt` leaves a 32-bit DIB section in, and it is also
/// [`BitmapPixelFormat::Bgra8`] — what the OCR engine takes — and also what
/// [`crate::page::Pixels`] reads and [`crate::page::Canvas`] draws into. So
/// the pixels copied off the screen are read, written over and shown again
/// without being converted once.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgrx: Vec<u8>,
}

/// The largest region that will be read, as a pixel count.
///
/// The whole of a 4K desktop is about 8 megapixels and takes an appreciable
/// moment to recognise; several monitors of it is not a translation request.
/// A region past this is read anyway — refusing would be worse — but it is
/// worth knowing the ceiling exists when the bubble seems slow.
const MAX_PIXELS: i64 = 16_000_000;

/// The language the screen is being read in, left here by the engine before
/// each request so that nothing below the platform line needs the config.
///
/// Empty, or "auto", means the user has not said: there is then nothing to
/// ask for and the reader falls back to the languages Windows was set up
/// with.
static READING: Mutex<String> = Mutex::new(String::new());

/// Tells the reader which language it is about to read.
pub fn set_reading_language(code: &str) {
    if let Ok(mut slot) = READING.lock() {
        let code = code.trim();
        let code = if code.eq_ignore_ascii_case("auto") {
            ""
        } else {
            code
        };
        code.clone_into(&mut slot);
    }
}

/// The recognition engine to read the screen with.
///
/// **Asked for the language being translated *from*, not the one Windows is
/// in.** `TryCreateFromUserProfileLanguages` reads with whatever the user's
/// own display language is, which is the wrong question: somebody reading an
/// English page on a Turkish install was handed the Turkish reader, and
/// somebody reading a Turkish page on an English one was handed the English
/// reader. Either way the engine answers — confidently, and with the wrong
/// letters, because a recogniser asked for the wrong language does not fail,
/// it guesses. Asking for the source language first is what makes it read
/// what is actually on the screen.
///
/// The user's own languages are still the answer when the source is left on
/// "detect it", and when the pack for the source is not installed — in which
/// case the window says so; see [`super::screen_reading`].
fn engine() -> Option<OcrEngine> {
    let wanted = READING
        .lock()
        .ok()
        .map(|code| code.clone())
        .unwrap_or_default();
    if !wanted.is_empty()
        && let Ok(language) = Language::CreateLanguage(&HSTRING::from(wanted.as_str()))
    {
        match OcrEngine::IsLanguageSupported(&language) {
            Ok(true) => match OcrEngine::TryCreateFromLanguage(&language) {
                Ok(engine) => return Some(engine),
                Err(error) => {
                    crate::trace!(
                        "ocr       no engine for {wanted} ({error}); using this computer's"
                    )
                }
            },
            _ => crate::trace!(
                "ocr       no {wanted} recognition pack installed; reading with {:?} instead",
                languages()
            ),
        }
    }
    match OcrEngine::TryCreateFromUserProfileLanguages() {
        Ok(engine) => Some(engine),
        Err(error) => {
            // The installed list comes with the failure, because that is the
            // question the failure raises: it is not that OCR is missing, it
            // is that the language the user reads in was never installed.
            crate::trace!(
                "ocr       no engine for the user's languages ({error}); installed: {:?}",
                languages()
            );
            None
        }
    }
}

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

    // Google Vision, when the user has entered a key of their own. It reads
    // faint text, small text and scripts the engine on the machine has no
    // pack for, and it reads them in about a second. The picture is sent to
    // Google to do it, which the window and the settings both say; without a
    // key nothing leaves the machine. Anything that goes wrong is not an
    // error anyone is shown — the screen is read here instead.
    if let Some(key) = crate::vision::key() {
        match crate::vision::read_text(&key, region.width as u32, region.height as u32, &pixels) {
            Ok(text) => return Some(crate::page::join_lines(&text)),
            Err(err) => crate::trace!("vision    {err}; reading here instead"),
        }
    }

    let bitmap = to_bitmap(&pixels, region.width, region.height).ok()?;
    let engine = engine()?;

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
    engine().is_some()
}

/// Whether the language being translated from has a recognition pack of its
/// own, which is the difference between reading a page and guessing at it.
///
/// `None` when the source is left on "detect it": there is no one language to
/// have a pack for, and the user's own are as good an answer as any.
pub fn source_language_installed() -> Option<bool> {
    let wanted = READING.lock().ok().map(|code| code.clone())?;
    if wanted.is_empty() {
        return None;
    }
    let language = Language::CreateLanguage(&HSTRING::from(wanted.as_str())).ok()?;
    Some(OcrEngine::IsLanguageSupported(&language).unwrap_or(false))
}

/// Copies `region` off the screen as a picture to read, write over and show.
///
/// The same `BitBlt` the rectangle gesture uses, kept as a [`Frame`] rather
/// than loose bytes because translating the whole screen passes the picture
/// through three hands — the reader, the painter and the window that shows it
/// — and each needs its size.
pub fn grab_frame(region: Region) -> Option<Frame> {
    let bgrx = grab(region)?;
    Some(Frame {
        width: region.width.max(0) as u32,
        height: region.height.max(0) as u32,
        bgrx,
    })
}

/// Every line of text in `frame`, and where each one is.
///
/// The other half of [`recognize`]: that answers what the text says, this
/// also answers where it was, which is what writing a translation back over
/// it needs. Google Vision when the user has set a key, the engine Windows
/// ships otherwise.
///
/// `None` when there is no reader at all. An empty list is a real answer.
pub fn recognize_lines(frame: &Frame, scale: f64) -> Option<Vec<crate::page::Line>> {
    if let Some(key) = crate::vision::key() {
        match crate::vision::read_words(&key, frame.width, frame.height, &frame.bgrx) {
            Ok(words) => return Some(crate::page::lines_from_words(words)),
            Err(err) => crate::trace!("vision    {err}; reading here instead"),
        }
    }
    // Read big and measured small: the boxes come back in the enlarged
    // picture's pixels, and everything after this — the ink the letters are
    // measured against, the place the translation is drawn — is in the
    // screen's own.
    let factor = enlargement(scale);
    if factor == 1 {
        return read_lines_here(frame);
    }
    let bigger = enlarged(frame, factor);
    crate::trace!(
        "ocr       reading {}x{} as {}x{}",
        frame.width,
        frame.height,
        bigger.width,
        bigger.height
    );
    let lines = read_lines_here(&bigger)?;
    Some(
        lines
            .into_iter()
            .map(|mut line| {
                line.rect = crate::page::Rect {
                    x: line.rect.x / factor,
                    y: line.rect.y / factor,
                    width: (line.rect.width / factor).max(1),
                    height: (line.rect.height / factor).max(1),
                };
                line
            })
            .collect(),
    )
}

/// How many times bigger the picture is made before it is read, for a display
/// that is not already drawing text at twice the size.
///
/// `Windows.Media.Ocr` learnt to read scans rather than screens, and on a
/// display at 100% a book's type is eight or nine pixels tall — small enough
/// that it reads "Cyber" as "cyber" and drops a line of a footnote entirely.
/// The same page at twice the size reads cleanly. A display already at 200%
/// hands over pixels that big to begin with, so there is nothing to gain and
/// four times the picture to read.
fn enlargement(scale: f64) -> u32 {
    if scale < 1.5 { 2 } else { 1 }
}

/// The picture at `factor` times the size, by repeating each pixel.
///
/// Nearest-neighbour rather than anything smoother on purpose: the reader is
/// looking for the edges of letters, and an interpolation that softens them
/// is the one thing that would undo the point of enlarging at all.
fn enlarged(frame: &Frame, factor: u32) -> Frame {
    let (width, height) = (frame.width * factor, frame.height * factor);
    let mut bgrx = Vec::with_capacity((width * height * 4) as usize);
    let mut row = Vec::with_capacity((width * 4) as usize);
    for y in 0..frame.height {
        row.clear();
        for x in 0..frame.width {
            let at = ((y * frame.width + x) * 4) as usize;
            for _ in 0..factor {
                row.extend_from_slice(&frame.bgrx[at..at + 4]);
            }
        }
        for _ in 0..factor {
            bgrx.extend_from_slice(&row);
        }
    }
    Frame {
        width,
        height,
        bgrx,
    }
}

/// Reads `frame` with the engine on the machine, keeping every word's box.
fn read_lines_here(frame: &Frame) -> Option<Vec<crate::page::Line>> {
    let longest = frame.width.max(frame.height);
    match OcrEngine::MaxImageDimension() {
        Ok(limit) if longest > limit => {
            // Not reachable through the key — one screen is captured, not the
            // whole desktop — but the engine refuses the picture outright
            // rather than reading part of it, and a silent nothing is worth a
            // line in the trace.
            crate::trace!("ocr       {longest}px is past the engine's {limit}px limit");
            return None;
        }
        _ => {}
    }

    let bitmap = to_bitmap(&frame.bgrx, frame.width as i32, frame.height as i32).ok()?;
    let engine = engine()?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|pending| pending.join())
        .ok()?;

    // A word's box arrives in the picture's own pixels, as floats, and is
    // trusted no further than that: a box that reaches past the edge would
    // index past the end of the pixels every measurement below reads.
    let clamp = |at: f32, limit: u32| (at.max(0.0) as u32).min(limit);
    let mut lines = Vec::new();
    for line in result.Lines().into_iter().flatten() {
        let mut words = Vec::new();
        for word in line.Words().into_iter().flatten() {
            let (Ok(box_), Ok(text)) = (word.BoundingRect(), word.Text()) else {
                continue;
            };
            let x = clamp(box_.X, frame.width);
            let y = clamp(box_.Y, frame.height);
            let rect = crate::page::Rect {
                x,
                y,
                width: clamp(box_.X + box_.Width, frame.width).saturating_sub(x),
                height: clamp(box_.Y + box_.Height, frame.height).saturating_sub(y),
            };
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            // `Windows.Media.Ocr` reports no confidence at all — there is no
            // such property on an `OcrWord` — so every word arrives unscored:
            // never doubted, and never counted as sure either. That costs two
            // of the three ways [`crate::page::lines_from_words`] throws out
            // an icon read as a letter; the third, a short token standing
            // taller than the words beside it, does not need a confidence and
            // still applies. What actually keeps a misread picture from being
            // "translated" is the check after it: whether the text, set in
            // the face that fits it, lands on the letters at all. See
            // [`crate::page::Pixels::replaceable`].
            words.push(crate::page::Word {
                rect,
                text: text.to_string_lossy(),
                confidence: crate::page::UNSCORED,
            });
        }
        if !words.is_empty() {
            lines.push(words);
        }
    }
    crate::trace!(
        "ocr       read {} lines off {}x{}",
        lines.len(),
        frame.width,
        frame.height
    );
    Some(crate::page::lines_from_words(lines))
}

/// Whether the whole screen can be read at all: an engine on the machine, or
/// a Google Vision key to send the picture to.
pub fn page_readable() -> bool {
    available() || crate::vision::key().is_some()
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
