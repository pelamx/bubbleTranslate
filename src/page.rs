//! Translating everything on the screen at once: Ctrl+Shift+F, Cmd+Shift+F
//! on a Mac.
//!
//! The screen is photographed, read with the same reader the rectangle uses
//! — but keeping where each line was — and every paragraph is written over
//! with its translation: on the lines it was on, in the face, size and colour
//! it was printed in. The result is shown as a still in place of the screen
//! until a key or a click puts it away.
//!
//! The rule everything here follows is that the picture is only ever changed
//! where there was text, and only where the change can be made cleanly. Text
//! on a photograph or a gradient, a word the reader was unsure of, a line
//! whose translation is itself — each is left exactly as it was. A screen
//! partly translated reads better than one with smudges on it.
//!
//! What lives here is the part every system shares: turning the reader's
//! lines into paragraphs, matching the original lettering, and drawing text
//! into the picture. Taking the picture, reading it and showing the result
//! are each platform's own.

use std::sync::OnceLock;

use ab_glyph::{Font, FontVec, GlyphId, ScaleFont};

/// A rectangle of the picture, in its own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}

/// One line of text as the reader found it: the box around its ink, and
/// what it says.
#[derive(Debug, Clone)]
pub struct Line {
    pub rect: Rect,
    pub text: String,
}

/// One word as the reader found it, with how sure it was of it out of 100.
#[derive(Debug, Clone)]
pub struct Word {
    pub rect: Rect,
    pub text: String,
    pub confidence: f32,
}

/// How sure the reader has to have been of a line, on average out of 100,
/// for it to be translated. Low on purpose: on a dark theme or in an
/// interface's own type the reader is unsure of words it read perfectly
/// well. What keeps a picture it tried to read from being "translated" is the
/// check that follows — whether the text, set in the matching face, lands on
/// the letters at all; see [`Pixels::replaceable`].
const MIN_CONFIDENCE: f32 = 30.0;

/// Turns the reader's lines of words into the lines that are translated.
///
/// Three things a reader gets wrong on a screen, put right here:
///
/// - It takes items that merely sit side by side for one line: three links
///   in a header, the tabs above an article. A gap wider than a space ends
///   one line and starts the next, so each is measured and replaced on its
///   own.
/// - It reads icons as letters: a magnifying glass as "Q,", a radio button
///   as "o". A token of a letter or two that stands taller than the words
///   beside it is an icon, and so is one with no letter or digit at all. Left
///   out, they are left on the screen as they were.
/// - It tries to read pictures. A line it was unsure of on the whole is left
///   alone.
pub fn lines_from_words(lines: Vec<Vec<Word>>) -> Vec<Line> {
    // Every word the reader found, kept whole for one last look below: what
    // sits just left of a line on its own row.
    let all: Vec<Word> = lines.iter().flatten().cloned().collect();
    let mut out = Vec::new();
    for words in lines {
        let mut heights: Vec<u32> = words.iter().map(|w| w.rect.height).collect();
        heights.sort_unstable();
        // The lower middle: of a word and an icon, the word's height.
        let Some(&median) = heights.get(heights.len().saturating_sub(1) / 2) else {
            continue;
        };
        // Cut only where the gap is plainly not a space: two letters wide or
        // more, as between tabs or the columns of a table. A fixed-width
        // face's space is a whole letter wide, and cutting a sentence there
        // translates its halves apart — into nonsense.
        let letters: usize = words.iter().map(|w| w.text.chars().count()).sum();
        let inked: u32 = words.iter().map(|w| w.rect.width).sum();
        let letter = inked as f32 / letters.max(1) as f32;
        let split_at = (letter * 2.2).max(median as f32 * 0.65);
        // Cut where the gaps are on the screen, before anything is taken out:
        // a word left out must not leave a gap that cuts its line in two.
        let mut pieces: Vec<Vec<Word>> = Vec::new();
        for word in words {
            match pieces.last_mut() {
                Some(piece)
                    if {
                        let last = piece.last().expect("a piece has a word");
                        let gap = word.rect.x as f32 - last.rect.right() as f32;
                        gap <= split_at
                    } =>
                {
                    piece.push(word)
                }
                _ => pieces.push(vec![word]),
            }
        }
        for piece in pieces {
            // An icon read as a letter or two stands taller than the words
            // beside it; a short word the reader was merely unsure of — "be",
            // "of" — does not, and stays, or its line is translated without
            // it and it sits there untranslated.
            // Code is recognised on the whole of each item, before anything
            // is taken out of it: `if` on its own is a word, and only beside
            // `translate_page(&cfg) {` is it plainly a program.
            // An icon at either end of an item — a button's glyph the reader
            // turned into "[£]" or "®" — is not part of what it says, and
            // must not make "Comment" look like code.
            let glyph = |w: &Word| {
                !w.text.chars().any(char::is_alphabetic)
                    && w.text
                        .chars()
                        .any(|c| "£©®¥@&§¶™°•●▪■□▶►✓✔✕✖★☆€$%".contains(c))
            };
            let words_only: Vec<&Word> = {
                let start = piece.iter().position(|w| !glyph(w)).unwrap_or(piece.len());
                let end = piece
                    .iter()
                    .rposition(|w| !glyph(w))
                    .map_or(start, |e| e + 1);
                piece[start..end.max(start)].iter().collect()
            };
            let whole = words_only
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if looks_like_code(&whole) {
                crate::trace!("page      left as it is, code: {whole:?}");
                continue;
            }
            let last_at = piece.len().saturating_sub(1);
            let piece: Vec<Word> = piece
                .into_iter()
                .enumerate()
                .filter(|(_, w)| w.text.chars().any(char::is_alphanumeric))
                .filter(|(at, w)| {
                    // A letter or two the reader was unsure of at either end of
                    // a line is a radio button, a bullet, an icon before a
                    // label; in the middle of a line a short word is a word.
                    // And at the start, anything with a character no word has
                    // in it is what is left of a key's chip or a button's
                    // border — "\:A'Lt" for an Alt key. Only at the start: at
                    // the end, a word the reader cut short at a column it
                    // imagined comes back as "grip|", and is still a word.
                    let edge = *at == 0 || *at == last_at;
                    let short = w.text.chars().count() <= 2;
                    if short && edge && w.confidence < 60.0 {
                        return false;
                    }
                    if *at == 0 && last_at > 0 && w.text.chars().any(|c| "\\|~^`".contains(c)) {
                        return false;
                    }
                    // A radio button or a bullet in front of a label, read
                    // as the letter it most looks like.
                    if *at == 0
                        && last_at > 0
                        && ["o", "O", "0", "©", "®", "•", "e", "@"].contains(&w.text.as_str())
                    {
                        return false;
                    }
                    // A bracket or a quote makes a token taller without
                    // making it an icon: "(9", "“A".
                    let punctuated = w.text.chars().any(|c| "()[]{}\"'“”‘’«»".contains(c));
                    // Taller than its neighbours is an icon only when it does
                    // not read as a word: "Q,", "3p", a lone letter. "By" and
                    // "On" stand taller than "the" because of their capitals.
                    let wordlike =
                        w.text.chars().count() == 2 && w.text.chars().all(char::is_alphabetic);
                    let taller = w.rect.height * 4 > median * 5 && !punctuated && !wordlike;
                    !(short && (taller || w.confidence < 30.0))
                })
                .map(|(_, w)| w)
                .collect();
            if piece.is_empty() {
                continue;
            }
            let mean = piece.iter().map(|w| w.confidence).sum::<f32>() / piece.len() as f32;
            if mean < MIN_CONFIDENCE {
                crate::trace!(
                    "page      left as it is, unsure ({mean:.0}%): {:?}",
                    piece
                        .iter()
                        .map(|w| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                continue;
            }
            let rect = piece
                .iter()
                .skip(1)
                .fold(piece[0].rect, |all, w| all.union(&w.rect));
            let text = piece
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            // The reader sometimes hands the start of a row over as a line of
            // its own — `1348 + //` before the comment it begins — and the
            // rest without it. Read together with what is just left of it on
            // its row, the rest is code again.
            let before = leading(&all, &rect);
            if !before.is_empty() && looks_like_code(&format!("{before} {text}")) {
                crate::trace!("page      left as it is, code: {before} {text:?}");
                continue;
            }
            out.push(Line { rect, text });
        }
    }
    out
}

/// Puts back together a line the reader cut in two.
///
/// A reader that lays a page out first sometimes sees a column break where
/// there is none — down the middle of a page of print — and hands each line
/// back as two, the cut sometimes inside a word: "just a g" and "impse of what
/// was". Translated apart they are nonsense, and set in two faces. Two pieces
/// on the same line, the same size, no further apart than a space, are one
/// line; items with a real gap between them — links, tabs, cells — stay apart.
pub fn join_split_lines(mut lines: Vec<Line>) -> Vec<Line> {
    let middle = |l: &Line| l.rect.y + l.rect.height / 2;
    let letter = |l: &Line| l.rect.width as f32 / l.text.chars().count().max(1) as f32;
    // Rows first — pieces whose middles are within a quarter of a line of
    // each other — and each row left to right: the half of a line read a
    // pixel higher than the other must still come after it.
    lines.sort_by_key(|line| middle(line));
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        match rows.last_mut() {
            Some(row)
                if row.iter().any(|other| {
                    middle(other).abs_diff(middle(&line)) * 4
                        <= other.rect.height.max(line.rect.height)
                }) =>
            {
                row.push(line)
            }
            _ => rows.push(vec![line]),
        }
    }
    let mut out = Vec::new();
    for mut row in rows {
        row.sort_by_key(|line| line.rect.x);
        let mut joined: Vec<Line> = Vec::new();
        for line in row {
            match joined.last_mut() {
                Some(before)
                    if {
                        let (a, b) = (&before.rect, &line.rect);
                        let gap = b.x as f32 - a.right() as f32;
                        a.height.max(b.height) * 10 <= a.height.min(b.height) * 13
                            && gap >= -2.0
                            && {
                                // Across a wide, justified space only two
                                // stretches of text are one line; a key's
                                // label or a button's — "Alt", "OK" — beside
                                // its description is two things, and joins
                                // only across an ordinary space.
                                let width = letter(before).max(letter(&line));
                                let shorter =
                                    before.text.chars().count().min(line.text.chars().count());
                                gap <= width * if shorter >= 4 { 2.2 } else { 1.0 }
                            }
                    } =>
                {
                    let gap = line.rect.x as f32 - before.rect.right() as f32;
                    // Cut inside a word: no space between the halves.
                    if gap >= letter(before) * 0.3 {
                        before.text.push(' ');
                    }
                    before.text.push_str(line.text.trim());
                    before.rect = before.rect.union(&line.rect);
                }
                _ => joined.push(line),
            }
        }
        out.extend(joined);
    }
    out
}

/// Lines read twice — by two strips of the screen that overlap — kept once:
/// of two that cover mostly the same place, the one with more text read, and
/// of two with as much, the one with the tighter box.
// Only the reader that cuts the screen into strips to read them at once needs
// this, which today is the Linux one; `Windows.Media.Ocr` reads a whole screen
// in one pass and reports each line once. The allow is so a reader that does
// not need it does not warn about one that does.
#[allow(dead_code)]
pub fn without_duplicates(mut lines: Vec<Line>) -> Vec<Line> {
    // Of two readings with as much in them, the tighter box: a line read near
    // the edge of a strip comes back in a box too big for it.
    lines.sort_by_key(|line| {
        (
            std::cmp::Reverse(line.text.chars().count()),
            line.rect.height,
        )
    });
    let mut kept: Vec<Line> = Vec::new();
    for line in lines {
        let same = kept.iter().any(|k| {
            let (a, b) = (&k.rect, &line.rect);
            let w = a.right().min(b.right()).saturating_sub(a.x.max(b.x));
            let h = a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y));
            let smaller = (a.width * a.height).min(b.width * b.height).max(1);
            w * h * 2 > smaller
        });
        if !same {
            kept.push(line);
        }
    }
    kept
}

/// What the reader found on the same row as `rect`, just left of it — at most
/// a few letters' width away — in order, as one string.
fn leading(all: &[Word], rect: &Rect) -> String {
    let middle = rect.y + rect.height / 2;
    let reach = rect.height * 4;
    let mut before: Vec<&Word> = all
        .iter()
        .filter(|w| {
            (w.rect.y..w.rect.bottom()).contains(&middle)
                && w.rect.right() <= rect.x
                && rect.x - w.rect.right() <= reach
        })
        .collect();
    // A button's icon is not the start of a program.
    before.retain(|w| {
        w.text.chars().any(char::is_alphanumeric)
            || !w
                .text
                .chars()
                .any(|c| "£©®¥@&§¶™°•●▪■□▶►✓✔✕✖★☆€$%[]".contains(c))
    });
    before.sort_by_key(|w| w.rect.x);
    // Only the run that reaches up to the line: a word further left with a
    // wide gap after it belongs to something else.
    let mut run: Vec<&Word> = Vec::new();
    let mut edge = rect.x;
    for w in before.into_iter().rev() {
        if edge - w.rect.right() > reach {
            break;
        }
        edge = w.rect.x;
        run.push(w);
    }
    run.reverse();
    run.iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// How big the letters of `line` are, from the box the reader drew round them.
///
/// The box hugs the ink, so how tall it is depends on which letters are in
/// it: "Weather" reaches from the top of the W to the baseline, "typing"
/// from the top of the t to the bottom of the g. A font's letters reach about
/// three quarters of its size above the baseline and a fifth below it.
fn letter_size(line: &Line) -> f32 {
    let descends = line
        .text
        .chars()
        .any(|c| matches!(c, 'g' | 'j' | 'p' | 'q' | 'y' | ',' | ';' | '(' | ')'));
    // Capitals, digits and the tall lowercase letters reach up to about three
    // quarters of the size; a word of only "sure" or "more" stops at half.
    let rises = line.text.chars().any(|c| {
        c.is_uppercase()
            || c.is_ascii_digit()
            || "bdfhiklt'\"!?/()[]{}|".contains(c)
            || (c.is_alphabetic() && !c.is_ascii())
    });
    let share = match (rises, descends) {
        (true, true) => 0.95,
        (true, false) => 0.74,
        (false, true) => 0.74,
        (false, false) => 0.53,
    };
    line.rect.height as f32 / share
}

/// A paragraph: the unit that is translated, and the lines its translation
/// is written back onto.
#[derive(Debug, Clone)]
pub struct Block {
    pub rect: Rect,
    pub text: String,
    pub lines: Vec<Line>,
}

/// Puts back together the lines that make one paragraph.
///
/// A sentence wrapped across three lines is one sentence, and translated in
/// three pieces it comes back as three fragments. So a line joins the
/// paragraph above it when it starts close underneath, is about the same
/// height, lines up with it — and, above all, when it was wrapped: when its
/// first word would not have fitted at the end of the line above. That last
/// test is what keeps a menu or a list apart, whose items sit as close as
/// wrapped lines do but each of which ended where it did on purpose.
///
/// `room` says how far right of a line the column it is in reaches — where
/// the empty paper beside it ends. Without a picture to ask, a line's own
/// right edge is the answer, and every close line is taken as wrapped.
///
/// A line with no letter in it — a clock, a page number, a row of dashes — has
/// nothing to translate and is left where it is.
pub fn paragraphs(mut lines: Vec<Line>, room: impl Fn(&Rect) -> u32) -> Vec<Block> {
    lines.retain(|line| {
        let code = looks_like_code(&line.text);
        if code {
            crate::trace!("page      left as it is, code: {:?}", line.text);
        }
        line.text.chars().any(char::is_alphabetic) && !code
    });
    lines.sort_by_key(|line| (line.rect.y, line.rect.x));

    // How far right each line's column reaches. Two answers, and the
    // narrower usually wins: the empty paper beside the line, and the
    // furthest any line lined up with it reaches. A sidebar's column ends
    // where its longest item does, well before the paper beside it does.
    // Only a column far wider than anything in it — a short list in a wide
    // panel — is measured by its paper.
    let rooms: Vec<u32> = lines
        .iter()
        .map(|line| {
            let r = &line.rect;
            let near = r.height.max(4);
            let aligned = lines
                .iter()
                .filter(|other| {
                    other.rect.x.abs_diff(r.x) <= near
                        && other.rect.y.abs_diff(r.y) <= near * 12
                        && other.rect.height.abs_diff(r.height) * 3 <= near
                })
                .map(|other| other.rect.right())
                .max()
                .unwrap_or(r.right());
            let paper = room(r).max(r.right());
            if paper.saturating_sub(aligned) > (aligned - r.x) / 2 {
                paper
            } else {
                aligned
            }
        })
        .collect();

    let mut open: Vec<(Block, u32)> = Vec::new();
    for (line, line_room) in lines.into_iter().zip(rooms) {
        let found = open.iter_mut().find(|(block, room)| {
            let last = block.lines.last().expect("a block has a line");
            continues(last, &line, *room)
        });
        match found {
            Some((block, room)) => {
                append(&mut block.text, line.text.trim());
                block.rect = block.rect.union(&line.rect);
                block.lines.push(line);
                *room = line_room;
            }
            None => open.push((
                Block {
                    rect: line.rect,
                    text: line.text.trim().to_string(),
                    lines: vec![line],
                },
                line_room,
            )),
        }
    }
    open.into_iter()
        .map(|(block, _)| block)
        .filter(|block| !(block.lines.len() == 1 && bare_name(&block.text)))
        .collect()
}

/// Whether `text`, standing on its own, is a name rather than a word: a file
/// or folder in a tree, a command, a setting — `src`, `target`, `linux`.
/// Interfaces capitalise their labels and sentences end in a full stop; a
/// bare lowercase word alone on its line does neither. Only ever asked of a
/// paragraph of one line: the last line of a wrapped sentence can be a bare
/// lowercase word too, and that one is language.
fn bare_name(text: &str) -> bool {
    let text = text.trim();
    !text.contains(' ')
        && text.chars().count() >= 2
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
}

/// Whether `text` is a line of code, a command or a path rather than a line
/// of language — which is left as it is: translated, `let total = 0;` comes
/// back as nonsense in the shape of a program.
///
/// Judged a line at a time, because a terminal or an editor holds both: a
/// program, and the sentences around it — a comment, an error message, an
/// assistant's answer — which are exactly what is worth translating. Prose
/// has a symbol now and then and few words joined by underscores; code is
/// made of them. So a line is code when enough of it is the punctuation
/// programs are written in, or when it carries two of the marks only code
/// has: an operator, a keyword starting it, an identifier, a path.
pub fn looks_like_code(text: &str) -> bool {
    // An editor or a diff puts a line number and a `+` or `-` in front of
    // the line; what it is, is decided by what follows them.
    let mut text = text.trim();
    loop {
        let rest = text
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['+', '-', '│', '|', '>'])
            .trim_start();
        if rest.len() == text.len() || rest.is_empty() {
            break;
        }
        text = rest;
    }
    // A comment in a program is part of the program: translated, it sits in
    // the code in another language, and in pieces.
    // Not "* ": far more often a bullet in front of a sentence than the
    // middle of a block comment.
    const COMMENTS: [&str; 6] = ["//", "/*", "# ", "--", "<!--", "\"\"\""];
    if COMMENTS.iter().any(|mark| text.starts_with(mark)) {
        return true;
    }
    // A dotfile: `.github`, `.cargo`.
    if !text.contains(' ')
        && text.starts_with('.')
        && text
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return true;
    }
    let visible: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if visible.is_empty() {
        return false;
    }
    let symbols = visible
        .iter()
        .filter(|c| "{}[];=<>_\\|&*#$`~^%@".contains(**c))
        .count();
    // Brackets and slashes are left out: prose has "(Beta)" and "and/or" too.
    if visible.len() >= 6 && symbols * 100 >= visible.len() * 15 {
        return true;
    }

    let mut marks = 0;
    const OPERATORS: [&str; 16] = [
        "::", "->", "=>", "==", "!=", "&&", "||", "+=", ":=", "</", "/>", "();", "){", ") {", "#!",
        "${",
    ];
    if OPERATORS.iter().any(|op| text.contains(op)) {
        marks += 1;
    }
    if text.ends_with(';') || text.ends_with('{') || text.ends_with('}') {
        marks += 1;
        // A statement on its own line — `return;`, `break;`, `}` — is
        // nothing but code.
        if !text.contains(' ') {
            marks += 1;
        }
    }
    // Words that only ever start code, and words that also start the second
    // line of a wrapped sentence — "for the next week", "return to the
    // menu" — which need another sign beside them.
    const ONLY_CODE: [&str; 20] = [
        "fn", "let", "const", "var", "def", "import", "pub", "struct", "impl", "function", "async",
        "#include", "package", "func", "sudo", "git", "cargo", "npm", "pip", "elif",
    ];
    const ALSO_PROSE: [&str; 9] = [
        "class", "from", "return", "use", "if", "else", "for", "while", "cd",
    ];
    // Past a shell's prompt, the command is what counts.
    let command = ["$ ", "> ", "# ", "% "]
        .iter()
        .find_map(|prompt| text.strip_prefix(prompt))
        .unwrap_or(text);
    let first = command.split_whitespace().next().unwrap_or("");
    let more = command.split_whitespace().count() > 1;
    if ONLY_CODE.contains(&first) && more {
        marks += 2;
    } else if ALSO_PROSE.contains(&first) && more {
        marks += 1;
    }
    if text.starts_with("$ ") || text.starts_with("> ") || text.starts_with("# ") {
        marks += 1;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let identifiers = words.iter().filter(|w| is_identifier(w)).count();
    if identifiers > 0 {
        marks += 1;
    }
    if identifiers * 3 >= words.len() {
        marks += 1;
    }
    marks >= 2
}

/// Whether `word` is a name only a program would use: `snake_case`,
/// `camelCase`, `a.b.c()`, a path or a file with an extension, a URL.
fn is_identifier(word: &str) -> bool {
    let word = word.trim_matches(|c: char| ",;:'\"()[]{}".contains(c));
    if word.chars().count() < 3 {
        return false;
    }
    let count = word.chars().count();
    let middle: String = word.chars().skip(1).take(count.saturating_sub(2)).collect();
    let inner = |c: char| middle.contains(c);
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    // `fetchUser`, `pagesTried`: two lowercase letters or more before the
    // first capital, and lowercase again after it. Not `eSignature`,
    // `iPhone` or `macOS`, which are names.
    let chars: Vec<char> = word.chars().collect();
    let camel = chars.windows(2).enumerate().any(|(at, pair)| {
        pair[0].is_lowercase()
            && pair[1].is_uppercase()
            && at >= 1
            && chars[..=at].iter().filter(|c| c.is_lowercase()).count() >= 2
            && chars[at + 2..].iter().any(|c| c.is_lowercase())
    });
    word.contains("://")
        || (inner('_') && letters > 1)
        || (middle.matches('/').count() >= 2 && letters > 1)
        || (camel && word.chars().next().is_some_and(char::is_lowercase))
        || (inner('.')
            && word.rsplit('.').next().is_some_and(|ext| {
                (1..=4).contains(&ext.len())
                    && ext
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == ')' || c == '(')
            }))
}

/// Whether `next` reads as the line after `last` in the same paragraph,
/// where `last`'s column reaches as far right as `room`.
fn continues(last: &Line, next: &Line, room: u32) -> bool {
    // Compared by the size of their letters, not the height of their boxes:
    // "sure." on the last line of a paragraph has a box half as tall as the
    // line above it, and is the same type.
    if letter_size(last).max(letter_size(next)) > letter_size(last).min(letter_size(next)) * 1.3 {
        return false;
    }
    let a = last.rect.height.max(1);
    // Starts below the middle of the last line, and less than a line's height
    // under it.
    let gap = i64::from(next.rect.y) - i64::from(last.rect.bottom());
    if gap < -(i64::from(a) / 2) || gap > i64::from(a) {
        return false;
    }
    // Set as close as text is set: the lines of a paragraph are rarely more
    // than one and three quarter times their letters' size apart, and the
    // items of a menu nearly always are.
    let pitch = next.rect.y.saturating_sub(last.rect.y) as f32;
    if pitch > 1.8 * letter_size(last).max(letter_size(next)) {
        return false;
    }
    // Lined up on the left, or sharing most of the narrower one's width:
    // centred text wraps without a common left edge.
    let (l, n) = (&last.rect, &next.rect);
    // The first line of a new paragraph, set in from the left after a line
    // that finished a sentence: print's way of starting a paragraph without
    // a blank line.
    let indented = n.x > l.x + a / 2;
    let finished = last
        .text
        .trim_end()
        .ends_with(['.', '!', '?', ':', '"', '”', '’', ')']);
    if indented && finished {
        return false;
    }
    let left_aligned = l.x.abs_diff(n.x) <= a * 2;
    let overlap = i64::from(l.right().min(n.right())) - i64::from(l.x.max(n.x));
    if !left_aligned && overlap * 2 <= i64::from(l.width.min(n.width)) {
        return false;
    }
    // Wrapped, or ended on purpose: had the first word of the next line fitted
    // after this one, it would have been put there.
    let chars = next.text.chars().count().max(1) as u32;
    let letter = n.width / chars;
    let first = next
        .text
        .split_whitespace()
        .next()
        .map_or(0, |word| word.chars().count() as u32);
    l.right() + letter * (first + 1) > room
}

/// Adds the next line of a paragraph to it, with a space between, or with
/// the hyphen taken out where a word was broken across the two.
pub fn append(current: &mut String, line: &str) {
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

/// A reader's plain text as paragraphs: the lines of each put back together,
/// a blank line between them.
///
/// What a reader hands back is lines, and a sentence wrapped across three of
/// them translated line by line comes back as three fragments. Every reader
/// that answers with text rather than boxes needs this, which is why it lives
/// here rather than beside one of them.
pub fn join_lines(raw: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in raw.lines().map(str::trim) {
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        append(&mut current, line);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs.join("\n")
}

/// The three kinds of lettering nearly all text on a screen is set in: the
/// sans-serif of interfaces and the web, the serif of documents and headings,
/// and the fixed width of terminals and code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sans,
    Serif,
    Mono,
}

/// One of the faces translations can be set in — a font on this machine —
/// and how much narrower than its own letters it is drawn.
///
/// Narrower, rather than smaller, is how a translation longer than its
/// original is made to fit where there is no room beside or below it: the eye
/// reads a letter's size from its height, which stays the original's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    index: usize,
    squeeze: f32,
}

impl Face {
    fn new(index: usize) -> Self {
        Self {
            index,
            squeeze: 1.0,
        }
    }

    /// The same face drawn `squeeze` times as wide.
    fn squeezed(self, squeeze: f32) -> Self {
        Self { squeeze, ..self }
    }

    /// Whether `self` and `other` are the same font, however squeezed.
    fn same_font(self, other: Face) -> bool {
        self.index == other.index
    }
}

/// A font a translation can be set in.
struct Loaded {
    font: FontVec,
    kind: Kind,
    bold: bool,
}

/// Fonts that come with every system and are always tried, as
/// `(path, index in a collection, kind, bold)`. Metric-compatible with Arial,
/// Times and Courier where possible, which is what much of the web and many
/// documents are set in. The fonts the desktop itself uses are found and
/// added beside them; see [`system_faces`].
#[cfg(target_os = "linux")]
const BUILT_IN: &[(&str, u32, Kind, bool)] = &[
    (
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        0,
        Kind::Sans,
        false,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        0,
        Kind::Sans,
        false,
    ),
    (
        "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        0,
        Kind::Sans,
        true,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        0,
        Kind::Sans,
        true,
    ),
    (
        "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
        0,
        Kind::Serif,
        false,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
        0,
        Kind::Serif,
        false,
    ),
    (
        "/usr/share/fonts/liberation/LiberationSerif-Bold.ttf",
        0,
        Kind::Serif,
        true,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
        0,
        Kind::Serif,
        true,
    ),
    (
        "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
        0,
        Kind::Mono,
        false,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        0,
        Kind::Mono,
        false,
    ),
    (
        "/usr/share/fonts/liberation/LiberationMono-Bold.ttf",
        0,
        Kind::Mono,
        true,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
        0,
        Kind::Mono,
        true,
    ),
    (
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        0,
        Kind::Sans,
        false,
    ),
    (
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        0,
        Kind::Sans,
        false,
    ),
];

/// What Windows draws its interface, its documents and its consoles in.
#[cfg(target_os = "windows")]
const BUILT_IN: &[(&str, u32, Kind, bool)] = &[
    (r"C:\Windows\Fonts\segoeui.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\segoeuib.ttf", 0, Kind::Sans, true),
    (r"C:\Windows\Fonts\segoeuisl.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\arial.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\arialbd.ttf", 0, Kind::Sans, true),
    (r"C:\Windows\Fonts\calibri.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\calibrib.ttf", 0, Kind::Sans, true),
    (r"C:\Windows\Fonts\tahoma.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\verdana.ttf", 0, Kind::Sans, false),
    (r"C:\Windows\Fonts\times.ttf", 0, Kind::Serif, false),
    (r"C:\Windows\Fonts\timesbd.ttf", 0, Kind::Serif, true),
    (r"C:\Windows\Fonts\georgia.ttf", 0, Kind::Serif, false),
    (r"C:\Windows\Fonts\cambria.ttc", 0, Kind::Serif, false),
    (r"C:\Windows\Fonts\consola.ttf", 0, Kind::Mono, false),
    (r"C:\Windows\Fonts\consolab.ttf", 0, Kind::Mono, true),
    (r"C:\Windows\Fonts\CascadiaMono.ttf", 0, Kind::Mono, false),
    (r"C:\Windows\Fonts\CascadiaCode.ttf", 0, Kind::Mono, false),
    (r"C:\Windows\Fonts\cour.ttf", 0, Kind::Mono, false),
];

/// What macOS draws its interface, its documents and its terminals in.
#[cfg(target_os = "macos")]
const BUILT_IN: &[(&str, u32, Kind, bool)] = &[
    ("/System/Library/Fonts/SFNS.ttf", 0, Kind::Sans, false),
    ("/System/Library/Fonts/Helvetica.ttc", 0, Kind::Sans, false),
    ("/System/Library/Fonts/Helvetica.ttc", 1, Kind::Sans, true),
    (
        "/System/Library/Fonts/HelveticaNeue.ttc",
        0,
        Kind::Sans,
        false,
    ),
    (
        "/System/Library/Fonts/HelveticaNeue.ttc",
        1,
        Kind::Sans,
        true,
    ),
    (
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        0,
        Kind::Sans,
        false,
    ),
    (
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        0,
        Kind::Sans,
        true,
    ),
    (
        "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
        0,
        Kind::Serif,
        false,
    ),
    (
        "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
        0,
        Kind::Serif,
        true,
    ),
    (
        "/System/Library/Fonts/Supplemental/Georgia.ttf",
        0,
        Kind::Serif,
        false,
    ),
    ("/System/Library/Fonts/SFNSMono.ttf", 0, Kind::Mono, false),
    ("/System/Library/Fonts/Menlo.ttc", 0, Kind::Mono, false),
    ("/System/Library/Fonts/Menlo.ttc", 1, Kind::Mono, true),
    ("/System/Library/Fonts/Monaco.ttf", 0, Kind::Mono, false),
    (
        "/System/Library/Fonts/Supplemental/Courier New.ttf",
        0,
        Kind::Mono,
        false,
    ),
];

/// The fonts this desktop actually uses, asked of fontconfig: the faces it
/// gives for "sans-serif", "monospace", the interface and so on — which are
/// what applications here draw with unless told otherwise — and, of the
/// families screens are most often set in, whichever are installed. Each in
/// its regular and bold weight.
///
/// This is what makes a terminal's translation come out in the terminal's own
/// font: whatever `monospace` is here, JetBrains Mono or another, is in the
/// list.
#[cfg(target_os = "linux")]
fn system_faces() -> Vec<(String, u32, Kind, bool)> {
    const GENERIC: [&str; 4] = ["sans-serif", "serif", "monospace", "system-ui"];
    const FAMILIES: [&str; 24] = [
        "Inter",
        "Adwaita Sans",
        "Adwaita Mono",
        "Cantarell",
        "Ubuntu",
        "Ubuntu Mono",
        "Noto Sans",
        "Noto Serif",
        "Noto Sans Mono",
        "DejaVu Sans",
        "DejaVu Serif",
        "DejaVu Sans Mono",
        "Roboto",
        "Open Sans",
        "Fira Sans",
        "JetBrains Mono",
        "JetBrainsMono Nerd Font",
        "Fira Code",
        "Fira Mono",
        "Cascadia Code",
        "CaskaydiaMono Nerd Font",
        "Source Code Pro",
        "Hack",
        "Iosevka",
    ];
    let ask = |pattern: &str| -> Option<(String, String, u32, Kind, bool)> {
        let out = std::process::Command::new("fc-match")
            .args([
                "-f",
                "%{family[0]}\t%{spacing}\t%{weight}\t%{index}\t%{file}",
                pattern,
            ])
            .output()
            .ok()?;
        let line = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut cols = line.split('\t');
        let family = cols.next()?.to_string();
        let spacing: u32 = cols.next()?.parse().unwrap_or(0);
        let weight: u32 = cols.next()?.parse().unwrap_or(80);
        let index: u32 = cols.next()?.parse().unwrap_or(0);
        let file = cols.next()?.trim().to_string();
        let lower = family.to_lowercase();
        let kind = if spacing >= 90 || lower.contains("mono") || lower.contains("code") {
            Kind::Mono
        } else if [
            "serif",
            "roman",
            "times",
            "georgia",
            "libertine",
            "garamond",
        ]
        .iter()
        .any(|word| lower.contains(word) && !lower.contains("sans"))
        {
            Kind::Serif
        } else {
            Kind::Sans
        };
        // Fontconfig's weights: 80 is regular, 200 bold.
        Some((family, file, index, kind, weight >= 180))
    };
    let mut found = Vec::new();
    for pattern in GENERIC {
        for weight in ["", ":bold"] {
            if let Some((_, file, index, kind, bold)) = ask(&format!("{pattern}{weight}")) {
                found.push((file, index, kind, bold));
            }
        }
    }
    for family in FAMILIES {
        for weight in ["", ":bold"] {
            // Fontconfig answers every question with something; only an
            // answer in the family asked for means it is installed.
            if let Some((name, file, index, kind, bold)) = ask(&format!("{family}{weight}"))
                && name.to_lowercase().replace(' ', "") == family.to_lowercase().replace(' ', "")
            {
                found.push((file, index, kind, bold));
            }
        }
    }
    found
}

#[cfg(not(target_os = "linux"))]
fn system_faces() -> Vec<(String, u32, Kind, bool)> {
    Vec::new()
}

/// The faces translations are drawn in, and the broad-coverage fonts the
/// bubble falls back to, for the letters a face does not have.
pub struct Fonts {
    faces: Vec<Loaded>,
    fallback: Vec<FontVec>,
}

/// How many faces are tried against each line: enough for every font a
/// desktop draws with, few enough that a screen is matched in a moment.
const MAX_FACES: usize = 28;

/// Loaded on the first page and kept: a CJK face is tens of megabytes, and
/// reading it again for every page would cost more than the page.
pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut wanted: Vec<(String, u32, Kind, bool)> = system_faces();
        wanted.extend(
            BUILT_IN
                .iter()
                .map(|(path, index, kind, bold)| (path.to_string(), *index, *kind, *bold)),
        );
        let mut seen = std::collections::HashSet::new();
        let mut faces = Vec::new();
        for (path, index, kind, bold) in wanted {
            if faces.len() == MAX_FACES || !seen.insert((path.clone(), index)) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if let Ok(font) = FontVec::try_from_vec_and_index(bytes, index) {
                crate::trace!(
                    "page      face {path}#{index} {kind:?}{}",
                    if bold { " bold" } else { "" }
                );
                faces.push(Loaded { font, kind, bold });
            }
        }
        Fonts {
            faces,
            fallback: crate::ui::theme::find_fallback_fonts()
                .into_iter()
                .filter_map(|(_, bytes)| FontVec::try_from_vec_and_index(bytes, 0).ok())
                .collect(),
        }
    })
}

impl Fonts {
    /// The faces this machine has.
    fn available(&self) -> impl Iterator<Item = Face> + '_ {
        (0..self.faces.len()).map(Face::new)
    }

    fn kind(&self, face: Face) -> Kind {
        self.faces.get(face.index).map_or(Kind::Sans, |f| f.kind)
    }

    fn bold(&self, face: Face) -> bool {
        self.faces.get(face.index).is_some_and(|f| f.bold)
    }

    /// The first face of `kind` and weight, for drawing text of our own.
    fn first(&self, kind: Kind, bold: bool) -> Option<Face> {
        self.faces
            .iter()
            .position(|f| f.kind == kind && f.bold == bold)
            .map(Face::new)
    }

    /// The face to draw in when nothing on the screen says which: the first
    /// regular sans-serif there is.
    fn plain(&self) -> Face {
        self.first(Kind::Sans, false).unwrap_or(Face::new(0))
    }

    /// The font that draws `c` in `face`, and its glyph: the face itself if it
    /// has the letter, the first fallback that does otherwise.
    fn glyph(&self, face: Face, c: char) -> Option<(&FontVec, GlyphId)> {
        self.faces
            .get(face.index)
            .map(|f| &f.font)
            .into_iter()
            .chain(self.fallback.iter())
            .find_map(|font| {
                let id = font.glyph_id(c);
                (id.0 != 0).then_some((font, id))
            })
    }

    fn advance(&self, face: Face, c: char, px: f32) -> f32 {
        self.glyph(face, c).map_or(0.0, |(font, id)| {
            font.as_scaled(px).h_advance(id) * face.squeeze
        })
    }

    fn width(&self, face: Face, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.advance(face, c, px)).sum()
    }

    /// Lays out `text` from `(x, baseline)` and hands every pixel of ink to
    /// `put`, with how much of it is covered.
    fn render(
        &self,
        face: Face,
        text: &str,
        px: f32,
        mut x: f32,
        baseline: f32,
        mut put: impl FnMut(i32, i32, f32),
    ) {
        let scale = ab_glyph::PxScale {
            x: px * face.squeeze,
            y: px,
        };
        for c in text.chars() {
            let Some((font, id)) = self.glyph(face, c) else {
                continue;
            };
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
            x += font.as_scaled(px).h_advance(id) * face.squeeze;
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outline.px_bounds();
            let (left, top) = (bounds.min.x as i32, bounds.min.y as i32);
            outline.draw(|gx, gy, coverage| put(left + gx as i32, top + gy as i32, coverage));
        }
    }

    /// The box around the ink of `text` set at size 1 from the origin, as
    /// `(left, top, right, bottom)` with the baseline at 0 and up negative.
    fn ink_box(&self, face: Face, text: &str) -> Option<(f32, f32, f32, f32)> {
        // Measured at a size where rounding to pixels costs nothing, and
        // scaled back.
        const AT: f32 = 100.0;
        let mut x = 0.0;
        let mut found: Option<(f32, f32, f32, f32)> = None;
        for c in text.chars() {
            let Some((font, id)) = self.glyph(face, c) else {
                continue;
            };
            let glyph = id.with_scale_and_position(AT, ab_glyph::point(x, 0.0));
            x += font.as_scaled(AT).h_advance(id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                found = Some(match found {
                    None => (b.min.x, b.min.y, b.max.x, b.max.y),
                    Some((l, t, r, bo)) => (
                        l.min(b.min.x),
                        t.min(b.min.y),
                        r.max(b.max.x),
                        bo.max(b.max.y),
                    ),
                });
            }
        }
        found.map(|(l, t, r, b)| (l / AT, t / AT, r / AT, b / AT))
    }
}

/// How a line of the original was set: the face and size that reproduce it,
/// where its baseline is, and its colours.
#[derive(Debug, Clone, Copy)]
struct Style {
    face: Face,
    px: f32,
    baseline: f32,
    paper: [u8; 3],
    /// Whether the paper is a gradient rather than one colour; cleared by
    /// carrying the gradient across, not by painting it over.
    smooth: bool,
    ink: [u8; 3],
    /// How well the face fitted, from 0 to 1.
    fit: f32,
}

/// How well a line's text, set in the face that fits it best, has to cover
/// its letters for it to be replaced; see [`Pixels::replaceable`].
const MIN_MATCH: f32 = 0.27;

/// [`MIN_MATCH`] for a line of forty letters or more.
const MIN_MATCH_LONG: f32 = 0.18;

/// A picture to read from: four bytes a pixel, blue first, top row first —
/// the layout every platform's screen capture already hands over.
pub struct Pixels<'a> {
    pub width: u32,
    pub height: u32,
    pub bgrx: &'a [u8],
}

/// How far two colours are apart, summed over the three channels.
fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(y))).sum()
}

/// How far from the paper a pixel may be and still count as paper: enough
/// for the dithering of a compressed screen, not enough for the faintest
/// grey a designer separates two panels with.
const PAPER_TOLERANCE: u32 = 12;

impl Pixels<'_> {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * self.width + x) * 4) as usize;
        [self.bgrx[at], self.bgrx[at + 1], self.bgrx[at + 2]]
    }

    /// The paper `rect` is printed on: the colour of a ring of pixels just
    /// outside it — or `None` when the ring is not one colour, because the
    /// text sits on a picture or a gradient that cannot be painted back.
    fn paper(&self, rect: Rect) -> Option<[u8; 3]> {
        let x0 = rect.x.saturating_sub(3);
        let y0 = rect.y.saturating_sub(3);
        let x1 = (rect.right() + 2).min(self.width.saturating_sub(1));
        let y1 = (rect.bottom() + 2).min(self.height.saturating_sub(1));
        let mut ring = Vec::new();
        for x in x0..=x1 {
            ring.push(self.pixel(x, y0));
            ring.push(self.pixel(x, y1));
        }
        for y in y0..=y1 {
            ring.push(self.pixel(x0, y));
            ring.push(self.pixel(x1, y));
        }
        // The commonest colour, not the average: a neighbour's letter
        // touching the ring must not tint the paper.
        let mut counts: Vec<([u8; 3], usize)> = Vec::new();
        for p in &ring {
            match counts.iter_mut().find(|(c, _)| distance(*c, *p) <= 3) {
                Some((_, n)) => *n += 1,
                None => counts.push((*p, 1)),
            }
        }
        let (paper, _) = counts.into_iter().max_by_key(|(_, n)| *n)?;
        let alike = ring
            .iter()
            .filter(|p| distance(**p, paper) <= PAPER_TOLERANCE)
            .count();
        // Nearly all of it: a few pixels of a neighbouring letter are allowed,
        // a picture or a gradient is not.
        (alike * 10 >= ring.len() * 9).then_some(paper)
    }

    /// The ring of pixels just outside `rect`, all the way round in order —
    /// along the top, down the right, back along the bottom, up the left.
    fn ring(&self, rect: Rect) -> Vec<[u8; 3]> {
        let x0 = rect.x.saturating_sub(3);
        let y0 = rect.y.saturating_sub(3);
        let x1 = (rect.right() + 2).min(self.width.saturating_sub(1));
        let y1 = (rect.bottom() + 2).min(self.height.saturating_sub(1));
        let mut ring = Vec::new();
        ring.extend((x0..=x1).map(|x| self.pixel(x, y0)));
        ring.extend((y0..=y1).map(|y| self.pixel(x1, y)));
        ring.extend((x0..=x1).rev().map(|x| self.pixel(x, y1)));
        ring.extend((y0..=y1).rev().map(|y| self.pixel(x0, y)));
        ring
    }

    /// The paper `rect` is printed on when it is not one colour but changes
    /// smoothly — a gradient, a tinted card — as the average of the ring
    /// round it; `None` when the ring has an edge in it, as a picture or a
    /// neighbouring box does.
    fn smooth_paper(&self, rect: Rect) -> Option<[u8; 3]> {
        let ring = self.ring(rect);
        if ring.len() < 8 {
            return None;
        }
        // From one pixel to the next round the ring the colour may drift,
        // never jump; a few jumps are a neighbouring letter's tip.
        let jumps = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .filter(|(a, b)| distance(**a, **b) > 15)
            .count();
        if jumps * 20 > ring.len() {
            return None;
        }
        let mut sum = [0u64; 3];
        for p in &ring {
            for (s, v) in sum.iter_mut().zip(p) {
                *s += u64::from(*v);
            }
        }
        Some(sum.map(|s| (s / ring.len() as u64) as u8))
    }

    /// The colour of the letters in `rect`: of every pixel in it, the one
    /// furthest from the paper. A link stays blue and a warning stays red.
    fn ink(&self, rect: Rect, paper: [u8; 3]) -> [u8; 3] {
        let mut best = (0, paper);
        for y in rect.y..rect.bottom().min(self.height) {
            for x in rect.x..rect.right().min(self.width) {
                let p = self.pixel(x, y);
                let d = distance(p, paper);
                if d > best.0 {
                    best = (d, p);
                }
            }
        }
        best.1
    }

    /// Whether every pixel of `rect` is `paper`.
    fn blank(&self, rect: Rect, paper: [u8; 3]) -> bool {
        if rect.right() > self.width || rect.bottom() > self.height {
            return false;
        }
        (rect.y..rect.bottom()).all(|y| {
            (rect.x..rect.right()).all(|x| distance(self.pixel(x, y), paper) <= PAPER_TOLERANCE)
        })
    }

    /// How far right of `rect` the paper stays empty: the first column, from
    /// its right edge on, where something on its strip is not `paper`.
    fn empty_until(&self, rect: Rect, paper: [u8; 3]) -> u32 {
        let top = rect.y.saturating_sub(1);
        let bottom = (rect.bottom() + 1).min(self.height);
        let mut x = rect.right() + 1;
        while x < self.width {
            let column_blank =
                (top..bottom).all(|y| distance(self.pixel(x, y), paper) <= PAPER_TOLERANCE);
            if !column_blank {
                return x;
            }
            x += 1;
        }
        self.width
    }

    /// `line` with its box shrunk to the ink inside it.
    ///
    /// A reader's box is only roughly where the letters are: on a dark theme
    /// or beside an icon it comes back half as tall again as they are, and
    /// measured against that no face fits. The picture says exactly where
    /// they are — every row and column of the box with something on it that
    /// is not paper.
    pub fn tighten(&self, mut line: Line) -> Line {
        let rect = line.rect;
        if rect.right() > self.width || rect.bottom() > self.height {
            return line;
        }
        let Some(paper) = self.paper(rect) else {
            return line;
        };
        let ink = self.ink(rect, paper);
        let contrast = distance(ink, paper);
        if contrast < 60 {
            return line;
        }
        let marked = |x: u32, y: u32| distance(self.pixel(x, y), paper) * 5 > contrast * 2;
        let rows: Vec<u32> = (rect.y..rect.bottom())
            .filter(|&y| (rect.x..rect.right()).any(|x| marked(x, y)))
            .collect();
        let columns: Vec<u32> = (rect.x..rect.right())
            .filter(|&x| (rect.y..rect.bottom()).any(|y| marked(x, y)))
            .collect();
        if let (Some(&top), Some(&bottom), Some(&left), Some(&right)) =
            (rows.first(), rows.last(), columns.first(), columns.last())
        {
            line.rect = Rect {
                x: left,
                y: top,
                width: right - left + 1,
                height: bottom - top + 1,
            };
        }
        line
    }

    /// How far right the column `rect` sits in reaches, for [`paragraphs`].
    pub fn column_edge(&self, rect: &Rect) -> u32 {
        match self.paper(*rect) {
            Some(paper) => self.empty_until(*rect, paper),
            None => rect.right(),
        }
    }

    /// Works out how `line` was set, by setting its text in each face this
    /// machine has, at the size that makes it as wide as the original, and
    /// keeping the one whose letters land where the original's did.
    ///
    /// `None` for a line that cannot be replaced cleanly: on a picture, or
    /// too faint to tell from its paper.
    fn style(&self, fonts: &Fonts, line: &Line) -> Option<Style> {
        self.matched(fonts, line).map(|(_, style)| style)
    }

    /// Whether `line` can be replaced: on plain paper, and read right — its
    /// text, set in the face that fits it best, lands where its letters are.
    /// A line the reader got wrong (an icon taken for letters, a phonetic
    /// spelling it has no letters for) does not, and is better left exactly
    /// as it is than replaced by the translation of what it was misread as.
    pub fn replaceable(&self, fonts: &Fonts, line: &Line) -> bool {
        let matched = self.matched(fonts, line).map(|(score, _)| score);
        // A long sentence with one word misread or missing still fits well
        // enough to be what it says; a short line that fits that badly is
        // something else — a logo, a phonetic spelling, an icon.
        let needed = if line.text.chars().count() >= 40 {
            MIN_MATCH_LONG
        } else {
            MIN_MATCH
        };
        let ok = matched.is_some_and(|score| score >= needed);
        if !ok {
            crate::trace!(
                "page      left as it is, {}: {:?}",
                match matched {
                    None => "not on plain paper".to_string(),
                    Some(score) => format!("read wrong (fit {score:.2})"),
                },
                line.text
            );
        }
        ok
    }

    /// [`Self::style`], with how well the face found fits, from 0 to 1.
    fn matched(&self, fonts: &Fonts, line: &Line) -> Option<(f32, Style)> {
        let rect = line.rect;
        if rect.width < 2 || rect.height < 4 || rect.right() > self.width {
            return None;
        }
        // Plain paper, or failing that a gradient: a coloured card, a
        // banner. A picture or anything with an edge in it is neither.
        let (paper, smooth) = match self.paper(rect) {
            Some(paper) => (paper, false),
            None => (self.smooth_paper(rect)?, true),
        };
        let ink = self.ink(rect, paper);
        let contrast = distance(ink, paper) as f32;
        if contrast < 60.0 {
            return None;
        }
        // How much of each pixel is ink, from 0 (paper) to 1.
        let (w, h) = (rect.width as usize, rect.height as usize);
        let mut original = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let p = self.pixel(rect.x + x as u32, rect.y + y as u32);
                original[y * w + x] = (distance(p, paper) as f32 / contrast).min(1.0);
            }
        }

        let text = line.text.trim();
        let mut best: Option<(f32, Style)> = None;
        let mut best_box = (0f32, 0f32, 0f32, 0f32);
        for face in fonts.available() {
            let Some((left, top, right, bottom)) = fonts.ink_box(face, text) else {
                continue;
            };
            let px = rect.width as f32 / (right - left).max(0.01);
            // Set as wide as the original, it must also be about as tall, or
            // this is not the face — or the reader misread the line.
            let tall = (bottom - top) * px / rect.height as f32;
            if !(0.6..=1.6).contains(&tall) {
                continue;
            }
            let tall_fit = if (0.75..=1.33).contains(&tall) {
                1.0
            } else {
                0.6
            };
            let baseline = rect.y as f32 - top * px;
            let origin = rect.x as f32 - left * px;
            let mut set = vec![0f32; w * h];
            fonts.render(face, text, px, origin, baseline, |x, y, coverage| {
                let (x, y) = (x - rect.x as i32, y - rect.y as i32);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    let at = y as usize * w + x as usize;
                    set[at] = (set[at] + coverage).min(1.0);
                }
            });
            // Two measures, because each is blind where the other sees. How
            // well the shapes overlap, blurred so that a fraction of a pixel
            // out of place does not count against a thin face — that tells a
            // serif from a sans. And how much ink there is, which no
            // misalignment changes — that tells bold from regular.
            let (soft_original, soft_set) = (blur(&original, w, h), blur(&set, w, h));
            let (mut both, mut either) = (0f32, 0f32);
            for (a, b) in soft_original.iter().zip(&soft_set) {
                both += a.min(*b);
                either += a.max(*b);
            }
            let (ink_original, ink_set) = (original.iter().sum::<f32>(), set.iter().sum::<f32>());
            let weight = ink_original.min(ink_set) / ink_original.max(ink_set).max(1.0);
            // A serif face has to match clearly better to be chosen: screens
            // are mostly sans-serif, and a sans line misread as serif looks
            // far more wrong than the other way round.
            // Bold likewise, by a little: in a fixed-width face it is exactly
            // as wide as the regular, and only a clearly better fit tells
            // them apart.
            let bias = match fonts.kind(face) {
                Kind::Serif => 0.8,
                Kind::Mono => 0.9,
                Kind::Sans => 1.0,
            } * if fonts.bold(face) { 0.95 } else { 1.0 };
            let score = bias * tall_fit * (both / either.max(1.0)) * weight;
            let style = Style {
                face,
                px,
                baseline,
                paper,
                smooth,
                ink,
                fit: score,
            };
            // Faces are tried in the desktop's own order, its defaults first,
            // and a later one has to fit clearly better to be chosen: two
            // sans-serifs fit body text about as well, and letting them take
            // turns line by line sets one paragraph in two faces and sizes.
            if best.is_none_or(|(s, _)| score > s * 1.08) {
                best = Some((score, style));
                best_box = (left, top, right, bottom);
            }
        }
        // The size, once the face is known. Its width alone says it wrongly
        // for a line with a misread letter or two; its height alone, for a
        // box a pixel too tall. Between the two the errors mostly cancel. A
        // fixed-width face keeps the width's answer: there the width is
        // exact, and it is what keeps columns in line.
        if let Some((_, style)) = best.as_mut() {
            let (_, top, _, bottom) = best_box;
            let by_height = rect.height as f32 / (bottom - top).max(0.01);
            if fonts.kind(style.face) != Kind::Mono {
                style.px = (style.px * by_height).sqrt();
            }
            style.baseline = rect.y as f32 - top * style.px;
        }
        // No face fitted: the reader's box is all there is. Its height is a
        // letter's size, near enough, and its bottom near the baseline.
        best.or_else(|| {
            let px = rect.height as f32 * 1.05;
            Some((
                0.0,
                Style {
                    face: fonts.plain(),
                    px,
                    baseline: rect.y as f32 + px * 0.76,
                    paper,
                    smooth,
                    ink,
                    fit: 0.0,
                },
            ))
        })
    }
}

/// `mask` (`w` by `h`) with every value averaged with its eight neighbours.
fn blur(mask: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            let mut n = 0.0;
            for ny in y.saturating_sub(1)..(y + 2).min(h) {
                for nx in x.saturating_sub(1)..(x + 2).min(w) {
                    sum += mask[ny * w + nx];
                    n += 1.0;
                }
            }
            out[y * w + x] = sum / n;
        }
    }
    out
}

/// Breaks `text` into rows, the first no wider than `widths[0]`, the second
/// than `widths[1]` and so on. `None` when it takes more rows than there are.
///
/// Each row is first filled only to its share in `targets` — the width the
/// original's line had, scaled by how much longer the translation is — so a
/// paragraph's translation spreads over all the lines it replaces instead of
/// crowding into the first ones and leaving the last ones empty. Past its
/// share a row takes a word only when there is no next row to put it on.
///
/// Between words where there are words. A word wider than a row by itself —
/// and in Chinese or Japanese, a whole sentence is one — is broken between
/// letters instead.
fn fit(
    fonts: &Fonts,
    face: Face,
    text: &str,
    px: f32,
    targets: &[f32],
    widths: &[f32],
) -> Option<Vec<String>> {
    let space = fonts.advance(face, ' ', px);
    let rows_total = widths.len();
    let limit = |row: usize| {
        if row + 1 >= rows_total {
            widths.get(row).copied().unwrap_or(0.0)
        } else {
            targets.get(row).copied().unwrap_or(0.0)
        }
    };
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0.0;
    for word in text.split_whitespace() {
        let w = fonts.width(face, word, px);
        let needed = if current.is_empty() {
            w
        } else {
            used + space + w
        };
        if needed <= limit(rows.len()) {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            used = needed;
            continue;
        }
        if !current.is_empty() && w <= limit(rows.len() + 1) {
            rows.push(std::mem::take(&mut current));
            current.push_str(word);
            used = w;
            if rows.len() >= rows_total {
                return None;
            }
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
            used += space;
        }
        for c in word.chars() {
            let advance = fonts.advance(face, c, px);
            if used + advance > widths.get(rows.len()).copied().unwrap_or(0.0)
                && !current.trim().is_empty()
            {
                rows.push(std::mem::take(&mut current));
                used = 0.0;
            }
            current.push(c);
            used += advance;
        }
        if rows.len() >= rows_total {
            return None;
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    (rows.len() <= rows_total).then_some(rows)
}

/// [`fit`] for a translation that does not fit even at the smallest size: as
/// much of it as the rows hold, ending in an ellipsis.
fn cut_short(fonts: &Fonts, face: Face, text: &str, px: f32, widths: &[f32]) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for keep in (0..words.len()).rev() {
        let shorter = format!("{}…", words[..keep].join(" "));
        if let Some(rows) = fit(fonts, face, &shorter, px, widths, widths) {
            return rows;
        }
    }
    Vec::new()
}

/// The sizes text is set in on a page: `sizes` grouped where they run on
/// with no jump between them, each group as `(smallest, largest, size)` with
/// the size its middle value.
fn size_groups(sizes: impl Iterator<Item = f32>) -> Vec<(f32, f32, f32)> {
    let mut sizes: Vec<f32> = sizes.collect();
    sizes.sort_by(f32::total_cmp);
    let mut groups: Vec<Vec<f32>> = Vec::new();
    for px in sizes {
        match groups.last_mut() {
            // A new size where the sizes jump, or where one group would
            // stretch too far: text measured a pixel apart is one size, a
            // heading a quarter bigger is another.
            Some(group) if px <= group[group.len() - 1] * 1.05 && px <= group[0] * 1.1 => {
                group.push(px)
            }
            _ => groups.push(vec![px]),
        }
    }
    // Two groups that meet with no gap between them are one size the cap
    // above happened to cut in two.
    let mut merged: Vec<Vec<f32>> = Vec::new();
    for group in groups {
        match merged.last_mut() {
            Some(last)
                if group[0] <= last[last.len() - 1] * 1.02
                    && group[group.len() - 1] <= last[0] * 1.12 =>
            {
                last.extend(group)
            }
            _ => merged.push(group),
        }
    }
    merged
        .into_iter()
        .map(|group| (group[0], group[group.len() - 1], group[group.len() / 2]))
        .collect()
}

/// The size of the group `px` belongs to.
fn snap(groups: &[(f32, f32, f32)], px: f32) -> f32 {
    groups
        .iter()
        .find(|(low, high, _)| (*low..=*high).contains(&px))
        .map_or(px, |(_, _, size)| *size)
}

/// How much smaller a label of one line may be drawn, once it is as narrow as
/// it may be and still does not fit. Never a paragraph.
const LABEL_SHRINK: [f32; 2] = [0.92, 0.85];

/// How much narrower than its own letters a translation may be drawn, tried in
/// turn until it fits.
const SQUEEZES: [f32; 5] = [1.0, 0.94, 0.88, 0.82, 0.76];

/// `rect` with `by` pixels more on every side.
fn grow(rect: Rect, by: u32) -> Rect {
    Rect {
        x: rect.x.saturating_sub(by),
        y: rect.y.saturating_sub(by),
        width: rect.width + 2 * by,
        height: rect.height + 2 * by,
    }
}

/// How many rows a translation may run on by, into empty paper under its
/// last line, when it needs more room than the original took. It is never
/// made smaller instead: text of one size on the original is one size in the
/// translation.
const EXTRA_ROWS: usize = 3;

/// Whether `translation` came back in a script that neither the original nor
/// the language asked for is written in — Chinese for a German line being
/// put into Turkish. That is the translator guessing at a misread line, not
/// translating it, and the line is better left as it was.
pub fn wrong_script(original: &str, translation: &str, target: &str) -> bool {
    let east = |c: char| {
        matches!(c as u32,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
    };
    let target = target.split('-').next().unwrap_or(target);
    if matches!(target, "zh" | "ja" | "ko") || original.chars().any(east) {
        return false;
    }
    let letters = translation.chars().filter(|c| c.is_alphabetic()).count();
    let eastern = translation.chars().filter(|c| east(*c)).count();
    eastern * 5 > letters.max(1)
}

/// Whether `translation` says what `original` already did — a name, a number,
/// a word that is the same in both languages. Such a line is left as it was:
/// the original's own pixels are the best rendering of it there is.
fn unchanged(original: &str, translation: &str) -> bool {
    let simple = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    simple(original) == simple(translation)
}

/// A picture to draw into, in the layout of [`Pixels`].
pub struct Canvas<'a> {
    pub width: u32,
    pub height: u32,
    pub bgrx: &'a mut [u8],
}

impl Canvas<'_> {
    fn pixels(&self) -> Pixels<'_> {
        Pixels {
            width: self.width,
            height: self.height,
            bgrx: self.bgrx,
        }
    }

    fn blend(&mut self, x: i32, y: i32, colour: [u8; 3], coverage: f32) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        let a = coverage.clamp(0.0, 1.0);
        for (channel, value) in colour.iter().enumerate() {
            let under = f32::from(self.bgrx[at + channel]);
            self.bgrx[at + channel] = (under + (f32::from(*value) - under) * a).round() as u8;
        }
    }

    /// Clears `rect` by carrying the paper round it across it: each pixel
    /// blended from the ring just outside, top and bottom, left and right,
    /// the way a gradient would have gone on had there been no text.
    fn fill_smooth(&mut self, rect: Rect) {
        let x0 = rect.x.saturating_sub(1);
        let y0 = rect.y.saturating_sub(1);
        let x1 = rect.right().min(self.width.saturating_sub(1));
        let y1 = rect.bottom().min(self.height.saturating_sub(1));
        if x1 <= x0 + 1 || y1 <= y0 + 1 {
            return;
        }
        let at = |c: &Self, x: u32, y: u32| -> [f32; 3] {
            let i = ((y * c.width + x) * 4) as usize;
            [c.bgrx[i] as f32, c.bgrx[i + 1] as f32, c.bgrx[i + 2] as f32]
        };
        let top: Vec<[f32; 3]> = (x0..=x1).map(|x| at(self, x, y0)).collect();
        let bottom: Vec<[f32; 3]> = (x0..=x1).map(|x| at(self, x, y1)).collect();
        let left: Vec<[f32; 3]> = (y0..=y1).map(|y| at(self, x0, y)).collect();
        let right: Vec<[f32; 3]> = (y0..=y1).map(|y| at(self, x1, y)).collect();
        let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
        let corners = [
            top[0],
            top[top.len() - 1],
            bottom[0],
            bottom[bottom.len() - 1],
        ];
        for y in y0 + 1..y1 {
            for x in x0 + 1..x1 {
                let (u, v) = ((x - x0) as f32 / w, (y - y0) as f32 / h);
                let (i, j) = ((x - x0) as usize, (y - y0) as usize);
                let i_at = ((y * self.width + x) * 4) as usize;
                for c in 0..3 {
                    // A Coons patch: the edges blended across, less the
                    // corners counted twice.
                    let value = (1.0 - v) * top[i][c]
                        + v * bottom[i][c]
                        + (1.0 - u) * left[j][c]
                        + u * right[j][c]
                        - ((1.0 - u) * (1.0 - v) * corners[0][c]
                            + u * (1.0 - v) * corners[1][c]
                            + (1.0 - u) * v * corners[2][c]
                            + u * v * corners[3][c]);
                    self.bgrx[i_at + c] = value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    fn fill(&mut self, rect: Rect, colour: [u8; 3]) {
        let right = rect.right().min(self.width);
        let bottom = rect.bottom().min(self.height);
        for y in rect.y..bottom {
            for x in rect.x..right {
                let at = ((y * self.width + x) * 4) as usize;
                self.bgrx[at..at + 3].copy_from_slice(&colour);
            }
        }
    }

    fn text(
        &mut self,
        fonts: &Fonts,
        face: Face,
        line: &str,
        at: (f32, f32),
        px: f32,
        ink: [u8; 3],
    ) {
        let mut ink_at = Vec::new();
        fonts.render(face, line, px, at.0, at.1, |x, y, coverage| {
            ink_at.push((x, y, coverage));
        });
        for (x, y, coverage) in ink_at {
            self.blend(x, y, ink, coverage);
        }
    }

    /// Writes every translation over its paragraph — [`Self::draw`] for each,
    /// with the sizes made the same across the page first.
    ///
    /// Measured one line at a time, two lines set in the same size come out a
    /// fraction of a pixel apart, and drawn at those sizes the translation
    /// looks set in a dozen. So the sizes found on the page are grouped —
    /// any within an eighth of each other are one size — and every line is
    /// drawn at its group's: body text at one size, headings at another, as
    /// the original was.
    pub fn draw_page(&mut self, fonts: &Fonts, translated: &[(Block, String)]) {
        // Everything is measured from the untouched picture first: clearing
        // one paragraph must not change what the next is measured against.
        let pixels = self.pixels();
        let measured: Vec<Option<Vec<Style>>> = translated
            .iter()
            .map(|(block, translation)| {
                if block.lines.is_empty() || unchanged(&block.text, translation) {
                    crate::trace!(
                        "page      left as it is, same in translation: {:?}",
                        block.text
                    );
                    return None;
                }
                let styles = block
                    .lines
                    .iter()
                    .map(|line| pixels.style(fonts, line))
                    .collect::<Option<Vec<Style>>>();
                if styles.is_none() {
                    crate::trace!(
                        "page      left as it is, not on plain paper: {:?}",
                        block.text
                    );
                }
                styles
            })
            .collect();
        let sizes = size_groups(measured.iter().flatten().flatten().map(|s| s.px));
        crate::trace!("page      sizes on the page: {sizes:?}");
        for ((block, translation), styles) in translated.iter().zip(measured) {
            let Some(mut styles) = styles else { continue };
            for style in &mut styles {
                style.px = snap(&sizes, style.px);
            }
            self.draw_measured(fonts, block, translation, styles);
        }
    }

    /// Writes `translation` over `block`, onto the lines its text was on, in
    /// the face, size and colours they were set in. Leaves the block exactly
    /// as it was when that cannot be done cleanly — see the module comment.
    #[cfg(test)]
    pub fn draw(&mut self, fonts: &Fonts, block: &Block, translation: &str) {
        self.draw_page(fonts, &[(block.clone(), translation.to_string())]);
    }

    /// [`Self::draw`], with the lines already measured.
    fn draw_measured(
        &mut self,
        fonts: &Fonts,
        block: &Block,
        translation: &str,
        styles: Vec<Style>,
    ) {
        let pixels = self.pixels();

        // One face and size for the paragraph: the original was set in one,
        // and a translation reflowed across its lines must not change size
        // from line to line.
        let mut sizes: Vec<f32> = styles.iter().map(|s| s.px).collect();
        sizes.sort_by(f32::total_cmp);
        let original = sizes[sizes.len() / 2];
        // Chosen by how much of the paragraph each face fits, and how well:
        // a long line that is plainly serif outweighs the two words of its
        // last line that fit a sans-serif as well as anything.
        let weight = |face: Face| -> f32 {
            styles
                .iter()
                .zip(&block.lines)
                .filter(|(s, _)| s.face.same_font(face))
                .map(|(s, line)| s.fit * line.text.chars().count() as f32)
                .sum()
        };
        let face = styles
            .iter()
            .map(|s| s.face)
            .max_by(|a, b| weight(*a).total_cmp(&weight(*b)))
            .unwrap_or(fonts.plain());

        // Where the rows go: the original lines, then up to two more in
        // empty paper under the last, a line's pitch apart.
        let mut rows: Vec<(Rect, Style)> = block
            .lines
            .iter()
            .map(|l| l.rect)
            .zip(styles.iter().copied())
            .collect();
        let last = *rows.last().expect("a block has a line");
        let pitch = if rows.len() > 1 {
            (last.1.baseline - rows[0].1.baseline) / (rows.len() - 1) as f32
        } else {
            original * 1.35
        };
        // A translation longer than its original runs on into the paper under
        // it — but only where that is empty all the way across, so it never
        // lands on anything else: under a tab or a cell of a table there is
        // the next one, and it stays where it is.
        let mut extra = Vec::new();
        let last_end = pixels.empty_until(last.0, last.1.paper);
        for k in 1..=EXTRA_ROWS {
            let offset = (pitch * k as f32).round() as u32;
            let below = Rect {
                y: last.0.y + offset,
                ..last.0
            };
            // Empty all the way across where its text would go, not only
            // under the last line's own words.
            let across = Rect {
                width: last_end.saturating_sub(below.x).max(below.width),
                ..below
            };
            if !pixels.blank(grow(across, 2), last.1.paper) {
                break;
            }
            let mut style = last.1;
            style.baseline += offset as f32;
            extra.push((below, style));
        }

        // How wide each row may be: on to the right as far as its paper is
        // empty, less a letter's breathing room before whatever is there.
        // A paragraph of several lines keeps to its own column: a justified
        // page's margin is empty paper too, and a translation running into
        // it no longer looks like the page. A line on its own may run on.
        let column = (block.lines.len() > 1).then(|| block.rect.right() + (original * 0.5) as u32);
        let available = |rect: Rect, style: &Style| -> f32 {
            let mut end = pixels.empty_until(rect, style.paper);
            if let Some(column) = column {
                end = end.min(column);
            }
            let margin = (style.px * 0.3) as u32;
            (end.saturating_sub(margin).max(rect.right()) - rect.x) as f32
        };
        let originals: Vec<f32> = rows.iter().map(|(r, _)| r.width as f32).collect();
        let widths: Vec<f32> = rows.iter().map(|(r, s)| available(*r, s)).collect();
        let extra_widths: Vec<f32> = extra.iter().map(|(r, s)| available(*r, s)).collect();

        let mut px = original;
        let attempt = |set_in: Face, with_extra: usize, px: f32| -> Option<Vec<String>> {
            let mut widths = widths.clone();
            widths.extend_from_slice(&extra_widths[..with_extra]);
            let ratio = fonts.width(set_in, translation, px)
                / fonts.width(face, &block.text, original).max(1.0);
            let mut targets: Vec<f32> = originals
                .iter()
                .zip(&widths)
                .map(|(o, w)| (o * ratio * 1.02).min(*w))
                .collect();
            targets.extend_from_slice(&extra_widths[..with_extra]);
            fit(fonts, set_in, translation, px, &targets, &widths)
        };
        let used_extra;
        // At the original's size always. Where it does not fit, first by
        // running on into the paper below; then, at each of those, a little
        // narrower — a letter's height is the size the eye reads, and up to a
        // quarter narrower it still reads as the same type.
        let mut set_in = face;
        let texts = 'found: {
            for squeeze in SQUEEZES {
                set_in = face.squeezed(squeeze);
                for with_extra in 0..=extra.len() {
                    if let Some(texts) = attempt(set_in, with_extra, px) {
                        used_extra = with_extra;
                        break 'found texts;
                    }
                }
            }
            // No room for all of it anywhere. A label of one line — a menu, a
            // tab, a button — is left as it was: cut to "D…" it says nothing
            // at all. A paragraph shows as much as fits, ending in an
            // ellipsis, at the original's size.
            // A label of one line gets one more chance: a little smaller,
            // which in a menu or on a button — set apart from everything
            // around it — the eye barely notices, where in a paragraph it
            // would.
            if block.lines.len() == 1 {
                for smaller in LABEL_SHRINK {
                    if let Some(texts) = attempt(set_in, extra.len(), original * smaller) {
                        px = original * smaller;
                        used_extra = extra.len();
                        break 'found texts;
                    }
                }
                crate::trace!(
                    "page      left as it is, no room: {:?} (room {:?}, needs {:.0} at {:.1}px)",
                    block.text,
                    widths,
                    fonts.width(face.squeezed(SQUEEZES[SQUEEZES.len() - 1]), translation, px),
                    px
                );
                return;
            }
            used_extra = extra.len();
            crate::trace!("page      cut short, no room: {:?}", block.text);
            let mut all = widths.clone();
            all.extend_from_slice(&extra_widths);
            cut_short(fonts, set_in, translation, px, &all)
        };
        rows.extend(extra.into_iter().take(used_extra));

        for ((rect, style), _) in rows.iter().zip(&block.lines) {
            if style.smooth {
                self.fill_smooth(grow(*rect, 2));
            } else {
                self.fill(grow(*rect, 2), style.paper);
            }
        }
        for ((rect, style), text) in rows.iter().zip(&texts) {
            self.text(
                fonts,
                set_in,
                text,
                (rect.x as f32, style.baseline),
                px,
                style.ink,
            );
        }
    }

    /// A line of small print across the bottom of the picture, on a dark
    /// strip of its own so it reads on any screen: how to put the picture
    /// away, and on the free tier how many trial pages are left.
    pub fn note(&mut self, fonts: &Fonts, text: &str, scale: f32) {
        let face = fonts.plain();
        let px = 15.0 * scale.max(1.0);
        let pad = (px * 0.8) as u32;
        let width = (fonts.width(face, text, px).ceil() as u32 + 2 * pad).min(self.width);
        let height = (px * 1.3).ceil() as u32 + pad;
        let rect = Rect {
            x: (self.width.saturating_sub(width)) / 2,
            y: self.height.saturating_sub(height + 3 * pad),
            width,
            height,
        };
        self.fill(rect, [40, 40, 40]);
        let baseline = rect.y as f32 + pad as f32 / 2.0 + px * 0.95;
        self.text(
            fonts,
            face,
            text,
            ((rect.x + pad) as f32, baseline),
            px,
            [240, 240, 240],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: u32, y: u32, width: u32, height: u32, text: &str) -> Line {
        Line {
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            text: text.into(),
        }
    }

    fn no_room(rect: &Rect) -> u32 {
        rect.right()
    }

    #[test]
    fn a_wrapped_paragraph_comes_back_as_one_block() {
        let blocks = paragraphs(
            vec![
                line(10, 10, 300, 16, "The quick brown fox"),
                line(10, 30, 280, 16, "jumps over the lazy"),
                line(10, 50, 100, 16, "dog."),
            ],
            no_room,
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text,
            "The quick brown fox jumps over the lazy dog."
        );
        assert_eq!(
            blocks[0].rect,
            Rect {
                x: 10,
                y: 10,
                width: 300,
                height: 56
            }
        );
        assert_eq!(blocks[0].lines.len(), 3);
    }

    #[test]
    fn a_gap_a_column_or_a_heading_starts_a_new_block() {
        let blocks = paragraphs(
            vec![
                // A heading twice the size of the text under it.
                line(10, 0, 200, 32, "Heading"),
                line(10, 40, 300, 16, "First paragraph."),
                // A blank line's worth of space above it.
                line(10, 90, 300, 16, "Second paragraph."),
                // The column beside, at the same height as the first.
                line(600, 40, 200, 16, "Sidebar item"),
            ],
            no_room,
        );
        let texts: Vec<_> = blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Heading",
                "First paragraph.",
                "Sidebar item",
                "Second paragraph."
            ]
        );
    }

    #[test]
    fn the_items_of_a_list_stay_apart() {
        // A menu in a column 300 wide: every item would have fitted after the
        // one above, so none of them was wrapped.
        let lines = vec![
            line(10, 10, 80, 14, "Geography"),
            line(10, 26, 70, 14, "History"),
            line(10, 42, 50, 14, "Politics"),
        ];
        let blocks = paragraphs(lines.clone(), |_| 310);
        assert_eq!(blocks.len(), 3);
        // The same lines in a column that ends where they do read as one
        // wrapped paragraph.
        assert_eq!(paragraphs(lines, no_room).len(), 1);
    }

    #[test]
    fn code_is_told_from_language() {
        for code in [
            "let total = items.iter().sum::<u32>();",
            "fn main() {",
            "import numpy as np",
            "$ cargo test --release",
            "if (user_id == null) return;",
            "src/platform/linux/ocr.rs:239",
            "const fetchUser = async (id) => {",
            "git commit -m \"Release 0.4.12\"",
            "<div class=\"card\">",
            "self.counter.pages_tried = 0",
            "~/.config/bubbleTranslate/config.toml",
            ".github",
            "install.sh",
            "return;",
            "// Only a paragraph grows: a label on its own",
            "# Install the packages first",
            "1352 - // Only a paragraph grows: a label on its own",
            "1180 +/// son satır, orijinalin kapladığından daha fazla",
            "+const EXTRA_ROWS: usize = 3;",
        ] {
            assert!(looks_like_code(code), "not seen as code: {code}");
        }
        for prose in [
            "The library is open every day from nine in the morning.",
            "Der Titel dieses Artikels ist mehrdeutig.",
            "I've found the problem: the old copy was still running.",
            "Prices rise again in October (see below).",
            "Bugünkü 10 ücretsiz çeviriyi kullandın",
            "Free trial: 2 of 3 left — unlimited with Pro.",
            "It works now. Try it again.",
            "Farbe (Beta)",
            "4 Request eSignature",
            "Open it on your iPhone or on macOS",
            "(Anfang)",
            "Mundarten/Sprachen",
            "Read and/or write the file.",
            "- a list item in a sentence",
            "3 files changed, 12 insertions",
            "Terminal",
            "Explorer",
            "dog.",
        ] {
            assert!(!looks_like_code(prose), "seen as code: {prose}");
        }
    }

    #[test]
    fn a_name_alone_is_left_alone_but_the_end_of_a_sentence_is_not() {
        // A file tree.
        let tree = paragraphs(
            vec![
                line(10, 10, 30, 11, "src"),
                line(10, 38, 50, 11, "target"),
                line(10, 66, 50, 11, "Explorer"),
            ],
            no_room,
        );
        let texts: Vec<_> = tree.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, ["Explorer"]);
        // A sentence whose last line is one lowercase word.
        let wrapped = paragraphs(
            vec![
                line(10, 10, 300, 16, "Press the key and try it"),
                line(10, 30, 50, 16, "again"),
            ],
            no_room,
        );
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].text, "Press the key and try it again");
    }

    #[test]
    fn a_short_word_the_reader_was_unsure_of_stays_in_its_line() {
        let word = |x: u32, width: u32, height: u32, text: &str, confidence: f32| Word {
            rect: Rect {
                x,
                y: 100 + 20 - height,
                width,
                height,
            },
            text: text.into(),
            confidence,
        };
        // "be" read at 40% — and "sure." with no tall letter, so a shorter box.
        let lines = lines_from_words(vec![vec![
            word(947, 40, 20, "one", 95.0),
            word(995, 60, 20, "could", 95.0),
            word(1064, 22, 20, "be", 40.0),
            word(1095, 51, 14, "sure.", 92.0),
        ]]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "one could be sure.");
        // And it joins the paragraph above it.
        let blocks = paragraphs(
            vec![
                line(947, 67, 896, 25, "one another: These days, no"),
                lines[0].clone(),
            ],
            no_room,
        );
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn a_line_cut_in_two_is_put_back_together() {
        let lines = join_split_lines(vec![
            line(
                947,
                564,
                681,
                25,
                "nobody could be sure the attack was over, or if it was just a g",
            ),
            line(1637, 564, 206, 25, "impse of what was"),
            // Links in a header, a gap of several letters apart, stay apart.
            line(919, 27, 90, 14, "Donate now"),
            line(1040, 27, 150, 14, "Create account"),
        ]);
        assert_eq!(
            lines.len(),
            3,
            "the header links stay two, the cut line is one"
        );
        assert!(
            lines[2].text.starts_with("nobody could") && lines[2].text.ends_with("of what was")
        );
        // A key's label beside its description stays apart.
        let chip = join_split_lines(vec![
            line(20, 150, 40, 22, "Alt"),
            line(90, 152, 400, 20, "select text, or double-click a word"),
        ]);
        assert_eq!(chip.len(), 2);
        let lines = join_split_lines(vec![
            line(
                990,
                531,
                639,
                25,
                "the time my plane touched down in Kyiv—in the dead",
            ),
            line(1645, 530, 200, 20, "of winter 2019—"),
        ]);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].text,
            "the time my plane touched down in Kyiv—in the dead of winter 2019—"
        );
    }

    #[test]
    fn an_indented_line_after_a_full_stop_starts_a_paragraph() {
        let blocks = paragraphs(
            vec![
                line(
                    947,
                    10,
                    680,
                    25,
                    "wisp of a model gripped my arm, and began to pray.",
                ),
                line(
                    985,
                    43,
                    640,
                    25,
                    "Three hundred feet below, Ukraine had gone into",
                ),
                line(
                    947,
                    76,
                    680,
                    25,
                    "orange alert. An abrupt windstorm was ripping",
                ),
            ],
            no_room,
        );
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].lines.len(), 2);
    }

    #[test]
    fn what_is_left_of_a_keys_chip_is_not_part_of_its_line() {
        let word = |x: u32, text: &str, confidence: f32| Word {
            rect: Rect {
                x,
                y: 100,
                width: text.chars().count() as u32 * 9,
                height: 18,
            },
            text: text.into(),
            confidence,
        };
        let lines = lines_from_words(vec![vec![
            word(10, "\\:A'Lt", 41.0),
            word(80, "select", 93.0),
            word(140, "text", 94.0),
        ]]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "select text");
        assert_eq!(lines[0].rect.x, 80);
    }

    #[test]
    fn a_line_without_letters_is_left_alone() {
        let blocks = paragraphs(
            vec![line(0, 0, 50, 16, "12:30"), line(0, 100, 50, 16, "Hi")],
            no_room,
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Hi");
    }

    #[test]
    fn a_broken_word_is_mended() {
        let mut text = String::from("transla-");
        append(&mut text, "tion");
        assert_eq!(text, "translation");
        let mut text = String::from("Jean-");
        append(&mut text, "Paul");
        assert_eq!(text, "Jean- Paul");
    }

    #[test]
    fn sizes_close_together_are_one_size() {
        let groups = size_groups([15.6, 16.0, 16.3, 15.9, 24.0, 23.2, 11.0].into_iter());
        assert_eq!(groups.len(), 3);
        // Every body line comes out the same, and the headings another.
        let body: Vec<f32> = [15.6, 16.0, 16.3]
            .iter()
            .map(|px| snap(&groups, *px))
            .collect();
        assert!(body.iter().all(|px| *px == body[0]));
        assert_eq!(snap(&groups, 23.2), snap(&groups, 24.0));
        assert_ne!(snap(&groups, 24.0), body[0]);
    }

    #[test]
    fn a_translation_in_an_unasked_for_script_is_refused() {
        assert!(wrong_script("Fehler bei", "错误发生在", "tr"));
        assert!(!wrong_script("Fehler bei", "Hata oluştu", "tr"));
        assert!(!wrong_script("Fehler bei", "错误发生在", "zh-CN"));
        assert!(!wrong_script("東京", "Tokyo", "en"));
    }

    #[test]
    fn a_translation_that_is_the_original_is_not_repainted() {
        assert!(unchanged("Bremen  ", "bremen"));
        assert!(!unchanged("Hauptstadt", "capital"));
    }

    /// Sets `text` in `face` on white paper, the way a screen would show it.
    fn printed(fonts: &Fonts, face: Face, text: &str, px: f32) -> (Vec<u8>, u32, u32, Line) {
        let (width, height) = (400u32, 80u32);
        let mut bgrx = vec![255u8; (width * height * 4) as usize];
        let mut canvas = Canvas {
            width,
            height,
            bgrx: &mut bgrx,
        };
        canvas.text(fonts, face, text, (20.0, 40.0), px, [30, 30, 30]);
        // The reader's box: the ink, exactly.
        let (l, t, r, b) = fonts.ink_box(face, text).unwrap();
        let rect = Rect {
            x: (20.0 + l * px).floor() as u32,
            y: (40.0 + t * px).floor() as u32,
            width: ((r - l) * px).ceil() as u32,
            height: ((b - t) * px).ceil() as u32,
        };
        (
            bgrx,
            width,
            height,
            Line {
                rect,
                text: text.into(),
            },
        )
    }

    #[test]
    fn the_original_face_and_size_are_recognised() {
        let fonts = fonts();
        for (kind, bold) in [
            (Kind::Sans, false),
            (Kind::Sans, true),
            (Kind::Serif, false),
            (Kind::Mono, false),
            (Kind::Mono, true),
        ] {
            let Some(face) = fonts.first(kind, bold) else {
                continue;
            };
            let (bgrx, width, height, line) = printed(fonts, face, "Weitere Informationen", 22.0);
            let pixels = Pixels {
                width,
                height,
                bgrx: &bgrx,
            };
            let style = pixels.style(fonts, &line).unwrap();
            // Another installed font of the same kind can fit as well — two
            // Arial-alikes are the same letters — but never one of another.
            assert_eq!(fonts.kind(style.face), kind, "{kind:?} bold={bold}");
            assert_eq!(fonts.bold(style.face), bold, "{kind:?} bold={bold}");
            assert!(
                (style.px - 22.0).abs() < 1.5,
                "{kind:?} read as {}",
                style.px
            );
            assert!((style.baseline - 40.0).abs() < 1.5);
            assert_eq!(style.ink, [30, 30, 30]);
        }
    }

    #[test]
    fn only_the_line_and_the_paper_after_it_are_painted() {
        let fonts = fonts();
        let Some(face) = fonts.first(Kind::Sans, false) else {
            return;
        };
        let (mut bgrx, width, height, line) = printed(fonts, face, "Guten Morgen", 18.0);
        // A grey mark beside the line: it may not be touched.
        let at = |x: u32, y: u32| ((y * width + x) * 4) as usize;
        let mark = at(380, line.rect.y + 2);
        bgrx[mark..mark + 3].copy_from_slice(&[128, 128, 128]);
        let before = bgrx.clone();
        let block = Block {
            rect: line.rect,
            text: line.text.clone(),
            lines: vec![line.clone()],
        };
        let mut canvas = Canvas {
            width,
            height,
            bgrx: &mut bgrx,
        };
        canvas.draw(fonts, &block, "Good morning");
        assert_ne!(bgrx, before, "the line was translated");
        let near = grow(line.rect, 2);
        for y in 0..height {
            for x in 0..width {
                let p = at(x, y);
                let on_strip = (near.y..near.bottom() + 6).contains(&y) && x < 370;
                if !on_strip {
                    assert_eq!(bgrx[p..p + 3], before[p..p + 3], "touched ({x}, {y})");
                }
            }
        }
        assert_eq!(bgrx[mark..mark + 3], [128, 128, 128]);
    }

    #[test]
    fn text_on_a_gradient_is_cleared_with_the_gradient() {
        let fonts = fonts();
        let Some(face) = fonts.first(Kind::Sans, false) else {
            return;
        };
        let (width, height) = (400u32, 80u32);
        // A soft left-to-right gradient, like a banner.
        let mut bgrx: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let v = 200 + ((i % width) * 50 / width) as u8;
                [v, v, v, 0]
            })
            .collect();
        let mut canvas = Canvas {
            width,
            height,
            bgrx: &mut bgrx,
        };
        canvas.text(
            fonts,
            face,
            "Guten Morgen",
            (20.0, 40.0),
            18.0,
            [20, 20, 20],
        );
        let (l, t, r, b) = fonts.ink_box(face, "Guten Morgen").unwrap();
        let line = Line {
            rect: Rect {
                x: (20.0 + l * 18.0).floor() as u32,
                y: (40.0 + t * 18.0).floor() as u32,
                width: ((r - l) * 18.0).ceil() as u32,
                height: ((b - t) * 18.0).ceil() as u32,
            },
            text: "Guten Morgen".into(),
        };
        let pixels = canvas.pixels();
        assert!(pixels.replaceable(fonts, &line), "a gradient is paper");
        let block = Block {
            rect: line.rect,
            text: line.text.clone(),
            lines: vec![line.clone()],
        };
        canvas.draw(fonts, &block, "Good morning");
        // Where the old letters were and the new ones are not, the gradient
        // is back: no dark leftovers, no flat grey patch.
        let right_end = line.rect.right() - 2;
        let at = ((line.rect.y + line.rect.height / 2) * width + right_end) as usize * 4;
        let expected = 200 + (right_end * 50 / width) as i32;
        assert!(
            (bgrx[at] as i32 - expected).abs() <= 6,
            "{} vs {expected}",
            bgrx[at]
        );
    }

    #[test]
    fn text_on_a_picture_is_left_alone() {
        let (width, height) = (60u32, 30u32);
        // A photograph: speckled, with edges everywhere — no paper to clear
        // it with, flat or graded.
        let mut bgrx: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let v = ((i.wrapping_mul(2_654_435_761) >> 13) % 256) as u8;
                [v, v, v, 0]
            })
            .collect();
        let before = bgrx.clone();
        let line = line(10, 10, 40, 12, "Hallo");
        let block = Block {
            rect: line.rect,
            text: line.text.clone(),
            lines: vec![line],
        };
        let mut canvas = Canvas {
            width,
            height,
            bgrx: &mut bgrx,
        };
        canvas.draw(fonts(), &block, "Hello");
        assert_eq!(bgrx, before);
    }

    #[test]
    fn a_translation_spreads_over_the_lines_it_replaces() {
        let fonts = fonts();
        let face = fonts.plain();
        let text = "one two three four five six seven eight";
        let half = fonts.width(face, text, 16.0) / 2.0 + 10.0;
        // Room for it all on the first row, but its share is half: it ends
        // up on both rows, as the original did.
        let rows = fit(fonts, face, text, 16.0, &[half, 400.0], &[400.0, 400.0]).unwrap();
        assert_eq!(rows.len(), 2);
        // More text than the rows hold does not fit, rather than spilling.
        assert!(fit(fonts, face, text, 16.0, &[60.0], &[60.0]).is_none());
        // Without spaces it still wraps, between letters.
        let rows = fit(
            fonts,
            face,
            "这是一个很长的中文句子没有空格",
            16.0,
            &[50.0; 10],
            &[50.0; 10],
        )
        .unwrap();
        assert!(rows.len() > 1);
        // Cut short, it ends in an ellipsis and stays on its rows.
        let rows = cut_short(fonts, face, text, 16.0, &[80.0]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].ends_with('…'));
    }

    /// Reads the PNG named by `BUBBLETRANSLATE_PAGE_SAMPLE`, translates it
    /// into `BUBBLETRANSLATE_PAGE_TARGET` (Turkish by default) and writes the
    /// painted result beside it as `-translated.png`: the whole path but the
    /// overlay, network included, which is why it is not part of the
    /// ordinary run.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn translates_a_sample_screen() {
        let path = std::env::var("BUBBLETRANSLATE_PAGE_SAMPLE").expect("a PNG to read");
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        let channels = info.line_size / info.width as usize;
        let mut bgrx = Vec::new();
        for px in buf[..info.buffer_size()].chunks(channels) {
            bgrx.extend_from_slice(&[px[2], px[1], px[0], 0]);
        }
        let (width, height) = (info.width, info.height);
        let started = std::time::Instant::now();
        let lines = crate::platform::recognize_lines_for_test(width, height, bgrx.clone());
        println!("{} lines read in {:?}", lines.len(), started.elapsed());
        if std::env::var_os("BUBBLETRANSLATE_PAGE_LINES").is_some() {
            let pixels = Pixels {
                width,
                height,
                bgrx: &bgrx,
            };
            for line in &lines {
                let score = pixels
                    .matched(fonts(), line)
                    .map(|(s, st)| (s, st.face.index, st.px));
                let code = looks_like_code(&line.text);
                println!("LINE {score:?} code={code} {:?} {:?}", line.text, line.rect);
            }
        }
        let pixels = Pixels {
            width,
            height,
            bgrx: &bgrx,
        };
        let lines = join_split_lines(lines)
            .into_iter()
            .map(|line| pixels.tighten(line))
            .filter(|line| pixels.replaceable(fonts(), line))
            .collect();
        let blocks = paragraphs(lines, |rect| pixels.column_edge(rect));
        assert!(!blocks.is_empty());
        let cfg = crate::config::Config {
            target_lang: std::env::var("BUBBLETRANSLATE_PAGE_TARGET")
                .unwrap_or_else(|_| "tr".into()),
            ..Default::default()
        };
        let texts: Vec<String> = blocks.iter().map(|b| b.text.clone()).collect();
        let started = std::time::Instant::now();
        let translated = crate::translate::Translator::new()
            .translate_many(&texts, &cfg)
            .unwrap();
        println!("{} paragraphs in {:?}", texts.len(), started.elapsed());
        let started = std::time::Instant::now();
        let mut canvas = Canvas {
            width,
            height,
            bgrx: &mut bgrx,
        };
        let pairs: Vec<(Block, String)> = blocks
            .into_iter()
            .zip(translated)
            .filter_map(|(block, text)| text.map(|text| (block, text)))
            .collect();
        canvas.draw_page(fonts(), &pairs);
        println!("painted in {:?}", started.elapsed());
        let out = path.replace(".png", "-translated.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&out).unwrap(), width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let rgb: Vec<u8> = bgrx.chunks(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgb)
            .unwrap();
        println!("wrote {out}");
    }

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
}
