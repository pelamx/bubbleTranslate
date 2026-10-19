//! The sentence a selected word stands in, read off the screen.
//!
//! macOS and Windows can ask the application what text surrounds a
//! selection. Linux cannot: the primary selection and data-control hand over
//! the selected text and nothing else, and the accessibility bus that could
//! say more is silent in terminals and off in browsers until asked. What
//! every application does have is pixels. So for a single word, a strip of
//! the screen around the pointer is photographed and read on this computer,
//! and the sentence is cut out of the line under the pointer and the lines
//! either side of it.
//!
//! The selected word itself is usually the one thing on that line the reader
//! does not read: it is drawn highlighted, light on a coloured band, and
//! Tesseract skips it. So it is not looked for. The words the reader did find
//! are laid out by where they are, and the selection is put back where the
//! pointer is — into the gap it left, or over whatever the reader made of it.
//!
//! Nothing here leaves the machine: the picture is read by Tesseract or the
//! built-in reader, never by Cloud Vision, and dropped once read. What goes to
//! Claude afterwards is the sentence, as it is on the other two systems.
//!
//! Only where the screen can be taken without asking — X11 and the wlroots
//! compositors. A desktop that takes screenshots through its portal may show
//! a dialog, which no one selecting a word expects; there the word is
//! translated on its own, as before.

use crate::page::Word;

use super::{ocr, screen};

/// How far the strip reaches either side of the pointer, in the pointer's
/// units: wide enough for a line of prose, narrow enough not to take in much
/// of a window beside it.
const HALF_WIDTH: f64 = 420.0;

/// How far it reaches above and below: the word's own line and two or three
/// either side, which is where the rest of its sentence is.
const HALF_HEIGHT: f64 = 56.0;

/// The sentence `word` stands in on the screen near `at`, or `None` when it
/// cannot be had: no reader, a screen that cannot be taken quietly, or no
/// line of text where the pointer is.
pub fn sentence_on_screen(word: &str, at: (f64, f64)) -> Option<String> {
    let started = std::time::Instant::now();
    let shot = match screen::grab_quietly() {
        Ok(shot) => shot,
        Err(err) => {
            crate::trace!("context   no picture: {err}");
            return None;
        }
    };
    let scale = shot.scale();
    let px = (at.0 - shot.origin.0) * scale;
    let py = (at.1 - shot.origin.1) * scale;
    if px < 0.0
        || py < 0.0
        || px >= f64::from(shot.frame.width)
        || py >= f64::from(shot.frame.height)
    {
        crate::trace!("context   the pointer is not on the screen photographed");
        return None;
    }
    let left = (px - HALF_WIDTH * scale).max(0.0) as u32;
    let top = (py - HALF_HEIGHT * scale).max(0.0) as u32;
    let right = ((px + HALF_WIDTH * scale) as u32).min(shot.frame.width);
    let bottom = ((py + HALF_HEIGHT * scale) as u32).min(shot.frame.height);
    let strip = shot.frame.crop(left, top, right - left, bottom - top);
    drop(shot);

    let lines = ocr::recognize_words_here(&strip, scale)?;
    drop(strip);
    let sentence = sentence_from_words(&lines, word, (px - f64::from(left), py - f64::from(top)));
    crate::trace!(
        "context   {} lines around the pointer, in {:?}: {:?}",
        lines.len(),
        started.elapsed(),
        sentence
    );
    sentence
}

/// Where a line of words is: its left and right, top and bottom.
fn span(words: &[Word]) -> Option<(f64, f64, f64, f64)> {
    let x0 = words.iter().map(|w| w.rect.x).min()?;
    let x1 = words.iter().map(|w| w.rect.x + w.rect.width).max()?;
    let y0 = words.iter().map(|w| w.rect.y).min()?;
    let y1 = words.iter().map(|w| w.rect.y + w.rect.height).max()?;
    Some((f64::from(x0), f64::from(x1), f64::from(y0), f64::from(y1)))
}

/// The word without what punctuation clings to it, in lower case: how a word
/// on the screen is compared with the selection.
fn bare(text: &str) -> String {
    text.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Cuts the sentence out of `lines` — the words read from a strip, line by
/// line — for `word`, selected with the pointer at `at` in the strip's
/// pixels. The line is the one under the pointer; on it, the word goes where
/// the pointer is, so a word that appears twice is read in the sentence it
/// was selected from.
fn sentence_from_words(lines: &[Vec<Word>], word: &str, at: (f64, f64)) -> Option<String> {
    let word = word.trim();
    let (px, py) = at;

    // The line under the pointer: the nearest by height, among those that
    // reach far enough sideways — a selection at either end of a line may be
    // the very word the reader left out, so a little beyond its ends counts.
    let (index, (x0, x1, y0, y1)) = lines
        .iter()
        .enumerate()
        .filter_map(|(i, words)| Some((i, span(words)?)))
        .filter(|(_, (x0, x1, y0, y1))| {
            let tall = y1 - y0;
            px >= x0 - tall * 4.0 && px <= x1 + tall * 4.0 && py >= y0 - tall && py <= y1 + tall
        })
        .min_by(|a, b| {
            let mid = |s: &(f64, f64, f64, f64)| ((s.2 + s.3) / 2.0 - py).abs();
            mid(&a.1).total_cmp(&mid(&b.1))
        })?;

    let mut words: Vec<&Word> = lines[index].iter().collect();
    words.sort_by_key(|w| w.rect.x);
    let right_of = |w: &Word| f64::from(w.rect.x + w.rect.width);
    let under = |w: &Word| f64::from(w.rect.x) <= px && px <= right_of(w);
    let want = bare(word);
    // Read after all, near the pointer: keep it as it was read, with its
    // punctuation. Read as something else under the pointer: that is the
    // selection, misread. Not read: it goes into the gap.
    let tall = y1 - y0;
    let found = words
        .iter()
        .enumerate()
        .filter(|(_, w)| bare(&w.text) == want)
        .filter(|(_, w)| f64::from(w.rect.x) - tall <= px && px <= right_of(w) + tall)
        .min_by(|a, b| {
            let d = |w: &Word| ((f64::from(w.rect.x) + right_of(w)) / 2.0 - px).abs();
            d(a.1).total_cmp(&d(b.1))
        })
        .map(|(i, w)| (i, i + 1, w.text.clone()));
    let (cut, resume, middle) = found
        .or_else(|| {
            let i = words.iter().position(|w| under(w))?;
            Some((i, i + 1, word.to_string()))
        })
        .unwrap_or_else(|| {
            let i = words
                .iter()
                .position(|w| (f64::from(w.rect.x) + right_of(w)) / 2.0 > px)
                .unwrap_or(words.len());
            (i, i, word.to_string())
        });

    // The lines of the same column, above and below: one beside it on the
    // screen, in another window or a sidebar, is not part of the sentence.
    let mut column: Vec<(f64, String)> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .filter_map(|(_, other)| {
            let (ox0, ox1, oy0, oy1) = span(other)?;
            (ox0 < x1.max(px) && ox1 > x0.min(px)).then(|| {
                let mut other: Vec<&Word> = other.iter().collect();
                other.sort_by_key(|w| w.rect.x);
                let text = other.iter().map(|w| w.text.as_str()).collect::<Vec<_>>();
                ((oy0 + oy1) / 2.0, text.join(" "))
            })
        })
        .collect();
    column.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mid = (y0 + y1) / 2.0;
    let join = |words: &[&Word]| {
        words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    let mut before: Vec<String> = column
        .iter()
        .filter(|(y, _)| *y < mid)
        .map(|(_, text)| text.clone())
        .collect();
    before.push(join(&words[..cut]));
    let mut after = vec![join(&words[resume..])];
    after.extend(
        column
            .iter()
            .filter(|(y, _)| *y > mid)
            .map(|(_, text)| text.clone()),
    );
    crate::context::sentence_around(
        &format!("{} ", before.join(" ")),
        &middle,
        &format!(" {}", after.join(" ")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Rect;

    /// A line of words set at `y`, each word 10 pixels a letter with a
    /// 10-pixel space, starting at `x`; `_` stands for a word the reader
    /// left out, which still takes its room.
    fn line(x: u32, y: u32, text: &str) -> Vec<Word> {
        let mut words = Vec::new();
        let mut at = x;
        for word in text.split(' ') {
            let width = word.chars().count() as u32 * 10;
            if !word.starts_with('_') {
                words.push(Word {
                    rect: Rect {
                        x: at,
                        y,
                        width,
                        height: 20,
                    },
                    text: word.into(),
                    confidence: 90.0,
                });
            }
            at += width + 10;
        }
        words
    }

    #[test]
    fn the_word_the_highlight_hid_goes_back_into_its_gap() {
        // "she now ____ a small company": "runs" was highlighted, and not read.
        let lines = [line(
            0,
            0,
            "After years at the firm she now ____ a small company in Izmir.",
        )];
        // "After years at the firm she now " is 32 letters: x = 320..360.
        assert_eq!(
            sentence_from_words(&lines, "runs", (340.0, 10.0)).as_deref(),
            Some("After years at the firm she now runs a small company in Izmir.")
        );
    }

    #[test]
    fn the_sentence_runs_across_the_lines_around_the_word() {
        let lines = [
            line(10, 0, "Rivers flood in spring. We sat on the"),
            line(10, 30, "____ of the river and watched the boats."),
            line(10, 60, "Then it rained."),
        ];
        assert_eq!(
            sentence_from_words(&lines, "bank", (30.0, 40.0)).as_deref(),
            Some("We sat on the bank of the river and watched the boats.")
        );
    }

    #[test]
    fn a_word_that_was_read_is_kept_as_it_was_read() {
        let lines = [line(0, 0, "Put the money in the bank, then go home.")];
        // "Put the money in the " is 21 letters: "bank," is x = 210..260.
        assert_eq!(
            sentence_from_words(&lines, "bank", (230.0, 10.0)).as_deref(),
            Some("Put the money in the bank, then go home.")
        );
    }

    #[test]
    fn a_misread_word_under_the_pointer_is_the_selection() {
        let lines = [line(0, 0, "We sat on the hauk of the river.")];
        assert_eq!(
            sentence_from_words(&lines, "bank", (150.0, 10.0)).as_deref(),
            Some("We sat on the bank of the river.")
        );
    }

    #[test]
    fn the_line_under_the_pointer_is_the_one_read() {
        let lines = [
            line(0, 0, "I put the money in the bank."),
            line(0, 100, "We sat on the ____ of the river."),
        ];
        assert_eq!(
            sentence_from_words(&lines, "bank", (150.0, 110.0)).as_deref(),
            Some("We sat on the bank of the river.")
        );
    }

    #[test]
    fn a_window_beside_it_is_not_part_of_the_sentence() {
        let lines = [
            line(0, 0, "We sat on the ____"),
            line(400, 0, "Settings Help"),
            line(400, 30, "About"),
            line(0, 30, "of the river."),
        ];
        assert_eq!(
            sentence_from_words(&lines, "bank", (150.0, 10.0)).as_deref(),
            Some("We sat on the bank of the river.")
        );
    }

    #[test]
    fn nothing_under_the_pointer_is_no_sentence() {
        let lines = [line(0, 0, "We sat on the bank of the river.")];
        assert_eq!(sentence_from_words(&lines, "bank", (150.0, 300.0)), None);
    }
}
