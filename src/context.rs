//! The sentence a selected word was taken from.
//!
//! One word translated on its own is a guess at which of its meanings was
//! meant: "bank" is a river's edge in one sentence and a building in the next,
//! and the phrase translators pick the commonest. The platforms that can say
//! what text sits around a selection hand it here, and this cuts out the
//! sentence the selection is in, for the translation to be made in the light
//! of it. See [`crate::translate::Translator::translate_selection`].

/// How much text either side of the word is kept at most, in characters. A
/// sentence is rarely longer; a line of a table with no full stop in it can be
/// much longer, and only the part near the word says anything about it.
const MAX_SIDE: usize = 240;

/// Where a sentence ends. A line break counts: the end of a line in a list,
/// a menu or a heading is the end of what the word belongs to.
fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？' | '\n' | '\r')
}

/// The sentence `word` sits in, given the text just `before` and `after` it.
/// `None` when there is nothing around the word worth sending: the word is
/// its own line, a label or a cell.
pub fn sentence_around(before: &str, word: &str, after: &str) -> Option<String> {
    let lead: String = {
        let tail: Vec<char> = before
            .chars()
            .rev()
            .take_while(|&c| !ends_sentence(c))
            .collect();
        let tail: String = tail.into_iter().rev().collect();
        trim_to_words(&tail, MAX_SIDE, true)
    };
    let rest: String = {
        let mut out = String::new();
        for c in after.chars() {
            out.push(c);
            if ends_sentence(c) {
                break;
            }
        }
        trim_to_words(out.trim_end_matches(['\n', '\r']), MAX_SIDE, false)
    };
    let sentence = format!("{lead}{word}{rest}");
    let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
    let alone = word.split_whitespace().collect::<Vec<_>>().join(" ");
    // A word with fewer than two others beside it has no sentence to read.
    if sentence.split_whitespace().count() < alone.split_whitespace().count() + 2 {
        return None;
    }
    Some(sentence)
}

/// At most `max` characters of `text`, cut at a space so no word is left in
/// half: the end nearest the word is the one kept.
fn trim_to_words(text: &str, max: usize, keep_end: bool) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if keep_end {
        let cut: String = text.chars().skip(count - max).collect();
        match cut.find(char::is_whitespace) {
            Some(at) => cut[at..].to_string(),
            None => cut,
        }
    } else {
        let cut: String = text.chars().take(max).collect();
        match cut.rfind(char::is_whitespace) {
            Some(at) => cut[..at].to_string(),
            None => cut,
        }
    }
}

/// The sentence around the first place `word` stands as a whole word in
/// `text`, for a platform that can give the sentence or paragraph a selection
/// is in but not where in it the selection starts.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sentence_containing(text: &str, word: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let is_letter = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut from = 0;
    while let Some(found) = text[from..].find(word) {
        let at = from + found;
        let end = at + word.len();
        if !is_letter(text[..at].chars().next_back()) && !is_letter(text[end..].chars().next()) {
            return sentence_around(&text[..at], &text[at..end], &text[end..]);
        }
        from = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_is_cut_out_of_the_paragraph() {
        let before = "We walked for hours. The boat drifted towards the ";
        let after = " of the river. Then it rained.";
        assert_eq!(
            sentence_around(before, "bank", after).as_deref(),
            Some("The boat drifted towards the bank of the river.")
        );
    }

    #[test]
    fn a_line_break_ends_the_sentence() {
        assert_eq!(
            sentence_around("Settings\nOpen the ", "file", " menu\nQuit").as_deref(),
            Some("Open the file menu")
        );
    }

    /// A word that is its own label has no sentence to send.
    #[test]
    fn a_word_on_its_own_has_no_sentence() {
        assert_eq!(sentence_around("Cancel\n", "Save", "\nHelp"), None);
        assert_eq!(sentence_around("", "Save", " as"), None);
    }

    #[test]
    fn a_very_long_side_is_cut_at_a_space() {
        let before = "word ".repeat(200);
        let sentence = sentence_around(&before, "bank", " here.").unwrap();
        assert!(sentence.chars().count() <= MAX_SIDE + 12);
        assert!(sentence.ends_with("bank here."));
        assert!(sentence.starts_with("word"));
    }

    #[test]
    fn the_word_is_found_as_a_whole_word() {
        let paragraph = "Embankments hold it. She sat on the bank and read. Banks close at five.";
        assert_eq!(
            sentence_containing(paragraph, "bank").as_deref(),
            Some("She sat on the bank and read.")
        );
        assert_eq!(sentence_containing(paragraph, "kayak"), None);
    }
}
