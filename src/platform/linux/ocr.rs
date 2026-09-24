//! Reading text out of pixels, with the Tesseract the user installed.
//!
//! Linux ships no recognition engine of its own the way Windows does, and
//! bundling one would mean carrying a model for every script anyone might
//! read — tens of megabytes per language, in a download that is otherwise one
//! small executable. Tesseract is packaged by every distribution, and its
//! languages are packaged one by one, so the person who reads Japanese
//! installs Japanese and nobody else pays for it.
//!
//! It is run as a command rather than linked. That keeps the binary starting
//! on a machine that has never heard of Tesseract — where the key that reads
//! the screen simply explains what to install — and keeps it free of a
//! library whose version differs on every distribution.
//!
//! Nothing leaves the machine here: the image is written to the user's own
//! runtime directory, read, and deleted.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// A picture of part of the screen, as the capture hands it over.
///
/// Four bytes a pixel, blue first — the order both X11 and the Wayland
/// capture deliver on a little-endian machine, so neither has to convert.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, B, G, R and one unused, top row first.
    pub bgrx: Vec<u8>,
}

impl Frame {
    /// The rectangle `(x, y, width, height)` of this frame, clamped to it.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Frame {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let mut bgrx = Vec::with_capacity((width * height * 4) as usize);
        for row in y..y + height {
            let start = ((row * self.width + x) * 4) as usize;
            bgrx.extend_from_slice(&self.bgrx[start..start + (width * 4) as usize]);
        }
        Frame {
            width,
            height,
            bgrx,
        }
    }

    /// The same picture at another size, by picking the nearest pixel.
    ///
    /// Only ever used to fit the frozen screen to the overlay window, where
    /// the two differ by a display scale and a sharper filter would buy
    /// nothing the eye can see in a dimmed backdrop.
    pub fn resized(&self, width: u32, height: u32) -> Frame {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let mut bgrx = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let from_row = (u64::from(row) * u64::from(self.height) / u64::from(height.max(1)))
                .min(u64::from(self.height.saturating_sub(1))) as u32;
            for col in 0..width {
                let from_col = (u64::from(col) * u64::from(self.width) / u64::from(width.max(1)))
                    .min(u64::from(self.width.saturating_sub(1)))
                    as u32;
                let at = ((from_row * self.width + from_col) * 4) as usize;
                bgrx.extend_from_slice(&self.bgrx[at..at + 4]);
            }
        }
        Frame {
            width,
            height,
            bgrx,
        }
    }

    /// Grey levels, one byte a pixel, `factor` times the size in each
    /// direction.
    ///
    /// Tesseract reads text best when a lowercase letter is a couple of dozen
    /// pixels tall, and on a display at 100% screen text is under ten. Its own
    /// rescaling does not make up for that, and a region read at its native
    /// size loses letters it gets right at twice the size — so a frame taken
    /// from an unscaled display is enlarged before it is read.
    fn grey(&self, factor: u32) -> Vec<u8> {
        let factor = factor.max(1);
        let out_width = (self.width * factor) as usize;
        let mut grey = Vec::with_capacity(out_width * (self.height * factor) as usize);
        let mut line = Vec::with_capacity(out_width);
        for row in 0..self.height {
            line.clear();
            for col in 0..self.width {
                let at = ((row * self.width + col) * 4) as usize;
                let [b, g, r] = [
                    u32::from(self.bgrx[at]),
                    u32::from(self.bgrx[at + 1]),
                    u32::from(self.bgrx[at + 2]),
                ];
                // Rec. 601 luma, in integers.
                let level = ((299 * r + 587 * g + 114 * b) / 1000) as u8;
                line.extend(std::iter::repeat_n(level, factor as usize));
            }
            for _ in 0..factor {
                grey.extend_from_slice(&line);
            }
        }
        grey
    }
}

/// How long a single recognition may take before it is given up on.
///
/// A small region reads in a few hundred milliseconds. A whole 4K screen of
/// dense text takes several seconds, and that is the worst anyone should be
/// asked to wait; past this something is wrong with the engine, and the
/// bubble is better off saying nothing than hanging the worker for good.
const BUDGET: Duration = Duration::from_secs(30);

/// Language packs that are not languages: orientation detection and the
/// maths model. Asking for either as a reading language only slows the read
/// down and makes it worse.
const NOT_LANGUAGES: [&str; 2] = ["osd", "equ"];

/// The installed reading languages, as Tesseract names them (`eng`, `tur`,
/// `jpn`…), or `None` when Tesseract is not installed at all.
///
/// Asked once per run. A language installed while the app is running is
/// picked up the next time it starts — the same as installing any other part
/// of the system it depends on.
pub fn languages() -> Option<&'static [String]> {
    static LANGUAGES: OnceLock<Option<Vec<String>>> = OnceLock::new();
    LANGUAGES
        .get_or_init(|| {
            let mut command = std::process::Command::new("tesseract");
            command.arg("--list-langs");
            let out = super::timed_output(command, Duration::from_secs(5)).ok()?;
            if !out.status.success() {
                return None;
            }
            // Older releases print the list on stderr, newer ones on stdout.
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            Some(parse_languages(&text))
        })
        .as_deref()
}

fn parse_languages(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        // The first line is a sentence about where the list came from.
        .filter(|line| !line.is_empty() && !line.contains(' ') && !line.ends_with(':'))
        .filter(|line| !NOT_LANGUAGES.contains(line))
        .map(str::to_string)
        .collect()
}

/// Why the screen cannot be read on this machine, in a sentence the user can
/// act on, or `None` when it can.
pub fn missing() -> Option<&'static str> {
    match languages() {
        None => Some(
            "Reading text off the screen needs Tesseract. Install the tesseract \
             package and a language for it — tesseract-data-eng, for example — \
             then restart bubbleTranslate.",
        ),
        Some([]) => Some(
            "Tesseract is installed but has no languages. Install one for the \
             text you read — tesseract-data-eng, for example — then restart \
             bubbleTranslate.",
        ),
        Some(_) => None,
    }
}

/// Reads whatever text is in `frame`.
///
/// `scale` is how many of the frame's pixels make one logical pixel of the
/// desktop: 2 on a HiDPI display, 1 on an ordinary one. It decides whether
/// the frame is enlarged first; see [`Frame::grey`].
///
/// `None` when the engine is missing or failed. An empty string is a real
/// answer — the pixels were read, and held no text.
pub fn recognize(frame: &Frame, scale: f64) -> Option<String> {
    let languages = languages()?;
    if languages.is_empty() {
        return None;
    }

    let factor = if scale < 1.5 { 2 } else { 1 };
    let (grey, width, height) = pad(
        &frame.grey(factor),
        frame.width * factor,
        frame.height * factor,
        MARGIN,
    );

    // A file rather than stdin, because the command runner that enforces the
    // deadline hands its child no input. PGM because it is a ten-byte header
    // in front of the pixels, which Tesseract reads without any help.
    let path = scratch_path();
    let mut pgm = format!("P5\n{width} {height}\n255\n").into_bytes();
    pgm.extend_from_slice(&grey);
    if let Err(err) = std::fs::write(&path, &pgm) {
        crate::trace!("ocr       could not write the image: {err}");
        return None;
    }

    let mut command = std::process::Command::new("tesseract");
    command
        .arg(&path)
        .arg("-")
        .arg("-l")
        .arg(languages.join("+"))
        // Tesseract's own threads make one read no faster in wall time and
        // several times more expensive in CPU; a single thread is its
        // documented advice for exactly this.
        .env("OMP_THREAD_LIMIT", "1");
    let started = std::time::Instant::now();
    let result = super::timed_output(command, BUDGET);
    // With tracing on, the last picture read is kept, so a region that came
    // back empty can be looked at rather than guessed about.
    if crate::trace::enabled() {
        let kept = path.with_file_name("bubbleTranslate-last-ocr.pgm");
        if std::fs::rename(&path, &kept).is_ok() {
            crate::trace!("ocr       kept the picture at {}", kept.display());
        }
    }
    let _ = std::fs::remove_file(&path);

    let out = match result {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            crate::trace!(
                "ocr       tesseract failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            return None;
        }
        Err(err) => {
            crate::trace!("ocr       tesseract did not run: {err}");
            return None;
        }
    };

    let text = join_lines(&String::from_utf8_lossy(&out.stdout));
    crate::trace!(
        "ocr       read {} chars from {width}x{height} as {} in {:?}",
        text.chars().count(),
        languages.join("+"),
        started.elapsed()
    );
    Some(text)
}

/// How much background is put around a region before it is read, in pixels.
///
/// A rectangle dragged tight around a line of text leaves the letters touching
/// the edge, and Tesseract reads those worse — measurably so, "It Is" for
/// "It is" on a line a margin fixes.
const MARGIN: u32 = 16;

/// `grey` with `margin` pixels added on every side, in the colour of its top
/// left corner, which is the background far more often than it is a letter.
fn pad(grey: &[u8], width: u32, height: u32, margin: u32) -> (Vec<u8>, u32, u32) {
    let background = grey.first().copied().unwrap_or(255);
    let (out_w, out_h) = (width + 2 * margin, height + 2 * margin);
    let mut out = vec![background; (out_w * out_h) as usize];
    for row in 0..height {
        let from = (row * width) as usize;
        let to = ((row + margin) * out_w + margin) as usize;
        out[to..to + width as usize].copy_from_slice(&grey[from..from + width as usize]);
    }
    (out, out_w, out_h)
}

/// Where the image waits for Tesseract: the user's runtime directory, which
/// only they can read and which is emptied at logout.
fn scratch_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("bubbleTranslate-ocr-{}.pgm", std::process::id()))
}

/// Turns Tesseract's layout back into sentences.
///
/// It reports each line on the screen as a line, and a gap between blocks as
/// an empty one. A sentence wrapped across three lines of a web page is still
/// one sentence, and a translator handed it in three pieces translates three
/// fragments — so lines are joined into paragraphs, and only the gaps
/// Tesseract saw survive as breaks. A word hyphenated across a line break is
/// put back together.
fn join_lines(raw: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in raw.lines().map(str::trim) {
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if current.is_empty() {
            current.push_str(line);
        } else if current.ends_with('-')
            && current
                .chars()
                .rev()
                .nth(1)
                .is_some_and(char::is_alphabetic)
            && line.chars().next().is_some_and(char::is_lowercase)
        {
            current.pop();
            current.push_str(line);
        } else {
            current.push(' ');
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_sentence_comes_back_as_one() {
        let raw = "The quick brown\nfox jumps over\nthe lazy dog.\n\nA second\nparagraph.\n";
        assert_eq!(
            join_lines(raw),
            "The quick brown fox jumps over the lazy dog.\nA second paragraph."
        );
    }

    #[test]
    fn a_word_broken_across_lines_is_mended() {
        assert_eq!(join_lines("transla-\ntion"), "translation");
        // A dash that is punctuation, not a break inside a word, stays.
        assert_eq!(join_lines("well -\nknown"), "well - known");
        assert_eq!(join_lines("Jean-\nPaul"), "Jean- Paul");
    }

    #[test]
    fn the_listing_header_and_non_languages_are_not_languages() {
        let listing = "List of available languages in \"/usr/share/tessdata/\" (4):\n\
                       eng\nosd\ntur\nequ\n";
        assert_eq!(parse_languages(listing), ["eng", "tur"]);
    }

    #[test]
    fn a_crop_is_the_pixels_it_names() {
        let frame = Frame {
            width: 3,
            height: 2,
            bgrx: (0..24).collect(),
        };
        let crop = frame.crop(1, 1, 5, 5);
        assert_eq!((crop.width, crop.height), (2, 1));
        assert_eq!(crop.bgrx, (16..24).collect::<Vec<u8>>());
    }

    #[test]
    fn a_margin_surrounds_the_picture_in_its_background() {
        let (out, w, h) = pad(&[9, 1, 1, 1], 2, 2, 1);
        assert_eq!((w, h), (4, 4));
        assert_eq!(out, [9, 9, 9, 9, 9, 9, 1, 9, 9, 1, 1, 9, 9, 9, 9, 9]);
    }

    #[test]
    fn enlarging_repeats_every_pixel() {
        let frame = Frame {
            width: 1,
            height: 1,
            bgrx: vec![0, 0, 255, 0],
        };
        let grey = frame.grey(2);
        assert_eq!(grey.len(), 4);
        assert!(grey.iter().all(|&level| level == grey[0]));
    }
}
