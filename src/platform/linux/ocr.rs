//! Reading text out of pixels: with the Tesseract the user installed, when
//! they did, and with the built-in reader otherwise.
//!
//! Linux ships no recognition engine of its own the way Windows does. The
//! built-in one ([`super::reader`]) needs nothing installed but knows only
//! unaccented Latin letters; carrying a model for every script anyone might
//! read would be tens of megabytes per language. Tesseract is the option
//! for the rest: packaged by every distribution, its languages packaged one
//! by one, so the person who reads Japanese installs Japanese and nobody else
//! pays for it. When it is installed with a language, it is the one used.
//!
//! It is run as a command rather than linked. That keeps the binary starting
//! on a machine that has never heard of Tesseract, and keeps it free of a
//! library whose version differs on every distribution.
//!
//! Nothing leaves the machine here: the image is written to the user's own
//! runtime directory, read, and deleted.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

    /// The picture as a reader gets it: grey, enlarged on an unscaled
    /// display, with a margin around it. `scale` is how many of the frame's
    /// pixels make one logical pixel of the desktop — 2 on a HiDPI display.
    pub(super) fn prepared(&self, scale: f64) -> (Vec<u8>, u32, u32) {
        let factor = enlargement(scale);
        let (width, height) = (self.width * factor, self.height * factor);
        let mut grey = self.grey(factor);
        light_paper(&mut grey, width, height);
        pad(&grey, width, height, MARGIN)
    }
}

/// How big a square of the picture is judged light or dark on its own, in
/// the prepared picture's pixels.
const TILE: u32 = 48;

/// Turns light text on a dark background into dark text on a light one, and
/// every background white, a square at a time.
///
/// Both readers learnt to read print — dark on light — and read a dark theme
/// badly: an editor's sidebar, a terminal, a dark web page came back with
/// half its words below the confidence they are kept at. Judged square by
/// square, from what most of each is, so a dark sidebar beside a light page
/// is turned and the page is not.
fn light_paper(grey: &mut [u8], width: u32, height: u32) {
    let mut ty = 0;
    while ty < height {
        let mut tx = 0;
        while tx < width {
            let (x1, y1) = ((tx + TILE).min(width), (ty + TILE).min(height));
            let mut histogram = [0u32; 256];
            for y in ty..y1 {
                for x in tx..x1 {
                    histogram[grey[(y * width + x) as usize] as usize] += 1;
                }
            }
            // The commonest level is the paper; letters are the minority.
            let paper = histogram
                .iter()
                .enumerate()
                .max_by_key(|(_, n)| **n)
                .map_or(255, |(level, _)| level);
            let dark = paper < 110;
            // And every square's paper made white, whatever shade it was: a
            // reader picks one level to split ink from paper by, and on a
            // screen of white panels and grey ones that level turns a whole
            // grey panel — a highlighted message, a card — solid black.
            let paper = if dark { 255 - paper } else { paper } as u32;
            for y in ty..y1 {
                for x in tx..x1 {
                    let at = (y * width + x) as usize;
                    let level = if dark { 255 - grey[at] } else { grey[at] } as u32;
                    grey[at] = (level * 255 / paper.max(1)).min(255) as u8;
                }
            }
            tx += TILE;
        }
        ty += TILE;
    }
}

/// How many times bigger a frame is made before it is read; see
/// [`Frame::grey`].
fn enlargement(scale: f64) -> u32 {
    if scale < 1.5 { 2 } else { 1 }
}

/// A point of the prepared picture, back in the frame's own pixels.
pub(super) fn unprepare(at: i64, scale: f64) -> u32 {
    ((at - i64::from(MARGIN)).max(0) / i64::from(enlargement(scale))) as u32
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

/// How long an answer about the installed languages is trusted.
///
/// The main window asks on every frame it draws, and the person reading it
/// may be installing a language in a terminal beside it at that very moment:
/// the window should notice within a few seconds, without a restart, and
/// without running Tesseract sixty times a second.
const LANGUAGES_TTL: Duration = Duration::from_secs(3);

/// The installed reading languages, as Tesseract names them (`eng`, `tur`,
/// `jpn`…), or `None` when Tesseract is not installed at all.
pub fn languages() -> Option<Vec<String>> {
    static CACHE: Mutex<Option<(Instant, Option<Vec<String>>)>> = Mutex::new(None);

    if let Ok(cache) = CACHE.lock()
        && let Some((at, languages)) = cache.as_ref()
        && at.elapsed() < LANGUAGES_TTL
    {
        return languages.clone();
    }
    let languages = list_languages();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), languages.clone()));
    }
    languages
}

fn list_languages() -> Option<Vec<String>> {
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

/// Whether Tesseract is installed with at least one language, which makes
/// it the reader used.
fn tesseract_ready() -> bool {
    languages().is_some_and(|languages| !languages.is_empty())
}

/// Gets a reader ready before the screen is taken over: nothing to do when
/// Tesseract is there, the built-in reader's models fetched when it is not.
/// The reason it cannot be, otherwise.
pub fn prepare() -> Result<(), String> {
    if tesseract_ready() {
        return Ok(());
    }
    super::reader::ensure()
}

/// Reads whatever text is in `frame`.
///
/// `scale` is how many of the frame's pixels make one logical pixel of the
/// desktop: 2 on a HiDPI display, 1 on an ordinary one. It decides whether
/// the frame is enlarged first; see [`Frame::grey`].
///
/// `None` when no reader is ready or it failed. An empty string is a real
/// answer — the pixels were read, and held no text.
pub fn recognize(frame: &Frame, scale: f64) -> Option<String> {
    if let Some(key) = crate::vision::key() {
        match crate::vision::read_text(&key, frame.width, frame.height, &frame.bgrx) {
            Ok(text) => return Some(crate::page::join_lines(&text)),
            Err(err) => crate::trace!("vision    {err}; reading here instead"),
        }
    }
    let raw = if tesseract_ready() {
        tesseract(frame, scale, Output::Text)?
    } else {
        super::reader::recognize(frame, scale)?
    };
    Some(crate::page::join_lines(&raw))
}

/// Reads every line of text in `frame`, with where each one is, in the
/// frame's own pixels. For the whole screen, where the translation has to be
/// painted back where the text was.
pub fn recognize_lines(frame: &Frame, scale: f64) -> Option<Vec<crate::page::Line>> {
    if let Some(key) = crate::vision::key() {
        match crate::vision::read_words(&key, frame.width, frame.height, &frame.bgrx) {
            Ok(words) => return Some(crate::page::lines_from_words(words)),
            Err(err) => crate::trace!("vision    {err}; reading here instead"),
        }
    }
    if tesseract_ready() {
        Some(tesseract_in_bands(frame, scale))
    } else {
        super::reader::recognize_lines(frame, scale)
    }
}

/// How many strips a whole screen is cut into, each read by its own
/// Tesseract at the same time. One Tesseract uses one core, and a whole
/// screen read by one took eight to twelve seconds.
const BANDS: u32 = 4;

/// How far each strip reaches into the next, in the frame's pixels: enough
/// that every line of ordinary text is whole, with room around it, in at
/// least one of them.
const BAND_OVERLAP: u32 = 96;

/// Reads the whole of `frame` as [`BANDS`] strips at once, and puts their
/// lines back together. A line the strips overlap on can be read by both, and
/// a reader sometimes misses a line near the edge of one strip that the other
/// sees whole: every line is kept, and of two that are the same line, the
/// one with more read in it.
fn tesseract_in_bands(frame: &Frame, scale: f64) -> Vec<crate::page::Line> {
    let bands = if frame.height > BAND_OVERLAP * 8 {
        BANDS.min(
            std::thread::available_parallelism()
                .map_or(1, |n| n.get() as u32)
                .max(1),
        )
    } else {
        1
    };
    let height = frame.height;
    let started = std::time::Instant::now();
    let lines = std::thread::scope(|scope| {
        let reads: Vec<_> = (0..bands)
            .map(|band| {
                scope.spawn(move || {
                    let top = (height * band / bands).saturating_sub(BAND_OVERLAP);
                    let bottom = (height * (band + 1) / bands + BAND_OVERLAP).min(height);
                    let strip = frame.crop(0, top, frame.width, bottom - top);
                    let tsv = tesseract(&strip, scale, Output::Tsv { band })?;
                    let mut lines = parse_tsv(&tsv, scale);
                    // A line cut by the strip's own edge is not a line.
                    lines.retain_mut(|line| {
                        let cut = (line.rect.y == 0 && top > 0)
                            || (line.rect.bottom() + 1 >= bottom - top && bottom < height);
                        line.rect.y += top;
                        !cut
                    });
                    Some(lines)
                })
            })
            .collect();
        reads
            .into_iter()
            .filter_map(|read| read.join().ok().flatten())
            .flatten()
            .collect::<Vec<_>>()
    });
    let lines = crate::page::without_duplicates(lines);
    crate::trace!(
        "ocr       {} lines from {bands} strips in {:?}",
        lines.len(),
        started.elapsed()
    );
    with_large_type(frame, scale, lines)
}

/// Which strip number the reading of large type writes its picture as.
const LARGE_BAND: u32 = 50;

/// `lines`, with whatever is set in large type read again at half the size.
///
/// Tesseract reads letters best at the size of ordinary print, and badly at
/// the size of a title page or a poster: "THIS IS / HOW THEY / TELL ME"
/// came back with words missing and "HOW TAEY". At half the size those
/// letters are in its range. Only done when the screen has large type at all
/// — a line at least twice as tall as most — so an ordinary screen costs
/// nothing more; and only the large lines are taken from it, in place of the
/// full-size reading of them.
fn with_large_type(
    frame: &Frame,
    scale: f64,
    lines: Vec<crate::page::Line>,
) -> Vec<crate::page::Line> {
    let mut heights: Vec<u32> = lines.iter().map(|line| line.rect.height).collect();
    heights.sort_unstable();
    let Some(&median) = heights.get(heights.len() / 2) else {
        return lines;
    };
    // Large type is at least two and a half times the screen's usual size:
    // a reading box twice as tall as most is often only a loose one, round a
    // word set in the same type as its neighbours.
    if median == 0 || !lines.iter().any(|line| line.rect.height * 2 >= median * 5) {
        return lines;
    }
    let started = std::time::Instant::now();
    // Half the size the full reading saw: on a scaled display the picture
    // halved; on an unscaled one, the picture as it is rather than doubled.
    let (small, back) = if enlargement(scale) == 1 {
        (frame.resized(frame.width / 2, frame.height / 2), 2)
    } else {
        (frame.clone(), 1)
    };
    // A scale of 2 reads the picture at the size it is given.
    let Some(tsv) = tesseract(&small, 2.0, Output::Tsv { band: LARGE_BAND }) else {
        return lines;
    };
    let tall = median * 2;
    let large: Vec<crate::page::Line> = parse_tsv(&tsv, 2.0)
        .into_iter()
        .map(|mut line| {
            line.rect = crate::page::Rect {
                x: line.rect.x * back,
                y: line.rect.y * back,
                width: line.rect.width * back,
                height: line.rect.height * back,
            };
            line
        })
        .filter(|line| line.rect.height >= tall)
        .collect();
    let overlaps = |a: &crate::page::Rect, b: &crate::page::Rect| {
        let w = a.right().min(b.right()).saturating_sub(a.x.max(b.x));
        let h = a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y));
        w * h * 2 > (a.width * a.height).min(b.width * b.height).max(1)
    };
    // Like for like only: a line of large type replaces the full-size reading
    // of it, but a box that came back far taller than the full-size one is a
    // loose box round ordinary type, and the full-size reading stays.
    let large: Vec<crate::page::Line> = large
        .into_iter()
        .filter(|big| {
            !lines.iter().any(|line| {
                overlaps(&big.rect, &line.rect) && line.rect.height * 13 < big.rect.height * 10
            })
        })
        .collect();
    let mut kept: Vec<crate::page::Line> = lines
        .into_iter()
        .filter(|line| !large.iter().any(|big| overlaps(&big.rect, &line.rect)))
        .collect();
    crate::trace!(
        "ocr       read {} lines of large type again at half the size in {:?}",
        large.len(),
        started.elapsed()
    );
    kept.extend(large);
    kept
}

/// What Tesseract is asked to print.
#[derive(Clone, Copy)]
enum Output {
    Text,
    /// A table with a row per word and where it is, for one strip of the
    /// screen; see [`tesseract_in_bands`].
    Tsv {
        band: u32,
    },
}

/// Turns Tesseract's table into lines: its words grouped by the block,
/// paragraph and line it says they belong to, each line as wide as its words.
fn parse_tsv(tsv: &str, scale: f64) -> Vec<crate::page::Line> {
    let mut lines: Vec<((u32, u32, u32), Vec<crate::page::Word>)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        // level, page, block, paragraph, line, word, left, top, width,
        // height, confidence, text.
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let num = |i: usize| cols[i].parse::<i64>().unwrap_or(0);
        let key = (num(2) as u32, num(3) as u32, num(4) as u32);
        let x = unprepare(num(6), scale);
        let y = unprepare(num(7), scale);
        let right = unprepare(num(6) + num(8), scale);
        let bottom = unprepare(num(7) + num(9), scale);
        let word = crate::page::Word {
            rect: crate::page::Rect {
                x,
                y,
                width: right.saturating_sub(x).max(1),
                height: bottom.saturating_sub(y).max(1),
            },
            text: text.to_string(),
            confidence: cols[10].trim().parse::<f32>().unwrap_or(0.0),
        };
        match lines.last_mut() {
            Some((last, words)) if *last == key => words.push(word),
            _ => lines.push((key, vec![word])),
        }
    }
    crate::page::lines_from_words(lines.into_iter().map(|(_, words)| words).collect())
}

fn tesseract(frame: &Frame, scale: f64, output: Output) -> Option<String> {
    let band = match output {
        Output::Text => 0,
        Output::Tsv { band } => band + 1,
    };
    let languages = languages()?;
    let (grey, width, height) = frame.prepared(scale);

    // A file rather than stdin, because the command runner that enforces the
    // deadline hands its child no input. PGM because it is a ten-byte header
    // in front of the pixels, which Tesseract reads without any help.
    let path = scratch_path(band);
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
        .arg(languages.join("+"));
    if matches!(output, Output::Tsv { .. }) {
        command.arg("tsv");
    }
    command
        // Tesseract's own threads make one read no faster in wall time and
        // several times more expensive in CPU; a single thread is its
        // documented advice for exactly this.
        .env("OMP_THREAD_LIMIT", "1");
    let started = std::time::Instant::now();
    let result = super::timed_output(command, BUDGET);
    // With tracing on, the last picture read is kept, so a region that came
    // back empty can be looked at rather than guessed about.
    if crate::trace::enabled() {
        let kept = path.with_file_name(match band {
            0 => "bubbleTranslate-last-ocr.pgm".to_string(),
            band => format!("bubbleTranslate-last-ocr-{band}.pgm"),
        });
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

    let text = String::from_utf8_lossy(&out.stdout).into_owned();
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
fn scratch_path(band: u32) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!(
        "bubbleTranslate-ocr-{}-{band}.pgm",
        std::process::id()
    ))
}

/// Tesseract's name for the language of an ISO 639-1 code, for the
/// languages the app offers. `None` for a code it has no pack for.
pub fn tesseract_code(iso: &str) -> Option<&'static str> {
    Some(match iso.split('-').next().unwrap_or(iso) {
        "en" => "eng",
        "tr" => "tur",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "pl" => "pol",
        "ru" => "rus",
        "uk" => "ukr",
        "ar" => "ara",
        "fa" => "fas",
        "hi" => "hin",
        "zh" => "chi_sim",
        "ja" => "jpn",
        "ko" => "kor",
        _ => return None,
    })
}

/// The families of distribution whose package names are known here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Arch,
    Debian,
    Fedora,
}

/// Which family this system belongs to, from `/etc/os-release`.
fn family() -> Option<Family> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    family_of(&text)
}

fn family_of(os_release: &str) -> Option<Family> {
    // ID first, then everything it says it is like: Mint is like Ubuntu,
    // which is like Debian; Rocky is like RHEL, which is like Fedora.
    let mut ids = Vec::new();
    for line in os_release.lines() {
        for key in ["ID=", "ID_LIKE="] {
            if let Some(value) = line.strip_prefix(key) {
                ids.extend(
                    value
                        .trim_matches('"')
                        .split_whitespace()
                        .map(str::to_lowercase),
                );
            }
        }
    }
    ids.iter().find_map(|id| match id.as_str() {
        "arch" | "manjaro" | "endeavouros" | "cachyos" => Some(Family::Arch),
        "debian" | "ubuntu" => Some(Family::Debian),
        "fedora" | "rhel" | "centos" => Some(Family::Fedora),
        _ => None,
    })
}

/// The command that installs what is missing for reading `pack`: Tesseract
/// itself when `with_engine`, and the language pack either way.
///
/// `None` on a distribution whose package names are not known here, where
/// the interface says what to install instead of how.
pub fn install_command(pack: &str, with_engine: bool) -> Option<String> {
    command_for(family()?, pack, with_engine)
}

fn command_for(family: Family, pack: &str, with_engine: bool) -> Option<String> {
    let (install, engine, language) = match family {
        Family::Arch => (
            "sudo pacman -S",
            "tesseract",
            format!("tesseract-data-{pack}"),
        ),
        // Debian's names use hyphens where Tesseract's use underscores.
        Family::Debian => (
            "sudo apt install",
            "tesseract-ocr",
            format!("tesseract-ocr-{}", pack.replace('_', "-")),
        ),
        Family::Fedora => (
            "sudo dnf install",
            "tesseract",
            format!("tesseract-langpack-{pack}"),
        ),
    };
    Some(if with_engine {
        format!("{install} {engine} {language}")
    } else {
        format!("{install} {language}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distribution_family_comes_from_id_or_what_it_is_like() {
        assert_eq!(family_of("ID=arch\n"), Some(Family::Arch));
        assert_eq!(
            family_of("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
            Some(Family::Debian)
        );
        assert_eq!(
            family_of("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"),
            Some(Family::Fedora)
        );
        assert_eq!(family_of("ID=nixos\n"), None);
    }

    #[test]
    fn each_family_names_its_own_packages() {
        assert_eq!(
            command_for(Family::Debian, "chi_sim", true).unwrap(),
            "sudo apt install tesseract-ocr tesseract-ocr-chi-sim"
        );
        assert_eq!(
            command_for(Family::Arch, "tur", false).unwrap(),
            "sudo pacman -S tesseract-data-tur"
        );
        assert_eq!(
            command_for(Family::Fedora, "eng", true).unwrap(),
            "sudo dnf install tesseract tesseract-langpack-eng"
        );
    }

    #[test]
    fn the_listing_header_and_non_languages_are_not_languages() {
        let listing = "List of available languages in \"/usr/share/tessdata/\" (4):\n\
                       eng\nosd\ntur\nequ\n";
        assert_eq!(parse_languages(listing), ["eng", "tur"]);
    }

    #[test]
    fn tesseracts_table_becomes_lines_in_the_frames_pixels() {
        // At scale 1 the picture was doubled and given a 16-pixel margin.
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   4\t1\t1\t1\t1\t0\t16\t16\t200\t30\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t16\t16\t80\t30\t95\tHello\n\
                   5\t1\t1\t1\t1\t2\t106\t20\t100\t26\t95\tworld\n\
                   5\t1\t1\t1\t1\t3\t240\t16\t20\t30\t31\t©\n\
                   5\t1\t1\t1\t2\t1\t16\t56\t60\t30\t95\tagain\n\
                   5\t1\t1\t1\t3\t1\t16\t96\t60\t30\t22\tg%}\n";
        let lines = parse_tsv(tsv, 1.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(
            lines[0].rect,
            crate::page::Rect {
                x: 0,
                y: 0,
                width: 95,
                height: 15
            }
        );
        assert_eq!(lines[1].text, "again");
        assert_eq!(lines[1].rect.y, 20);
    }

    #[test]
    fn dark_squares_are_turned_light_and_light_ones_left() {
        // Left half dark paper with a light letter, right half light paper.
        let (width, height) = (96u32, 48u32);
        let mut grey: Vec<u8> = (0..width * height)
            .map(|i| if i % width < 48 { 30 } else { 240 })
            .collect();
        grey[10] = 220;
        light_paper(&mut grey, width, height);
        // Both papers end up white, and the letter dark on it.
        assert_eq!(grey[0], 255);
        assert!(grey[10] < 50);
        assert_eq!(grey[60], 255);
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
