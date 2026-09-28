//! Which program reads text aloud here: `say`, which every Mac has, with a
//! voice picked for the language. Without one the text would be read in the
//! system's default voice, which reads Turkish as if it were English — so a
//! language with no voice installed gets no Listen button instead.

use std::process::Command;
use std::sync::OnceLock;

const SAY: &str = "/usr/bin/say";

/// Every installed voice as (name, locale), from `say -v ?`, whose lines read
/// `Yelda               tr_TR    # Merhaba, benim adım Yelda.` Names can have
/// spaces in them, so the locale is taken as the last word before the `#`.
fn voices() -> &'static [(String, String)] {
    static VOICES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    VOICES.get_or_init(|| {
        let Ok(out) = Command::new(SAY).args(["-v", "?"]).output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let head = line.split('#').next()?.trim_end();
                let (name, locale) = head.rsplit_once(char::is_whitespace)?;
                Some((name.trim().to_string(), locale.to_string()))
            })
            .collect()
    })
}

fn voice_for(lang: &str) -> Option<&'static str> {
    voices()
        .iter()
        .find(|(_, locale)| locale.split(['_', '-']).next() == Some(lang))
        .map(|(name, _)| name.as_str())
}

pub fn can_speak(lang: &str) -> bool {
    voice_for(lang).is_some()
}

pub fn command(text: &str, lang: &str) -> Option<Command> {
    let mut cmd = Command::new(SAY);
    // The leading space keeps a translation that starts with "-" from being
    // taken for an option.
    cmd.arg("-v").arg(voice_for(lang)?).arg(format!(" {text}"));
    Some(cmd)
}
