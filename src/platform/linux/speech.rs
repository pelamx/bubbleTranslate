//! Which program reads text aloud here.
//!
//! Nothing is bundled — the release stays a single binary — so this uses what
//! the desktop already has. Piper first, for any language it has a voice
//! downloaded for, because it is the one that sounds like a person rather than
//! a machine. Then Speech Dispatcher, because it is what the desktop's own
//! screen reader talks to and it speaks with whichever voice the user set up;
//! eSpeak on its own after that. With none of them installed the bubble simply
//! has no Listen button.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
enum Speaker {
    SpeechDispatcher,
    EspeakNg,
    Espeak,
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn speaker() -> Option<Speaker> {
    static FOUND: OnceLock<Option<Speaker>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        [
            ("spd-say", Speaker::SpeechDispatcher),
            ("espeak-ng", Speaker::EspeakNg),
            ("espeak", Speaker::Espeak),
        ]
        .into_iter()
        .find(|(name, _)| on_path(name).is_some())
        .map(|(_, s)| s)
    })
}

/// Piper's program, checked to be the speech one: `piper` is also the name
/// of a GTK tool for configuring gaming mice, which must not be started with
/// a voice model as its argument.
fn piper() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            ["piper-tts", "piper"]
                .into_iter()
                .filter_map(on_path)
                .find(|p| {
                    Command::new(p).arg("--help").output().is_ok_and(|o| {
                        let help = [o.stdout, o.stderr].concat();
                        String::from_utf8_lossy(&help).contains("--model")
                    })
                })
        })
        .as_deref()
}

/// What plays the WAV Piper writes: PipeWire's own player, else ALSA's.
fn player() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        ["pw-play", "paplay", "aplay"]
            .into_iter()
            .find(|p| on_path(p).is_some())
    })
}

/// Every Piper voice model on this machine. Piper has no voice directory of
/// its own, so these are the places a voice is put: the user's data
/// directory by hand, and `/usr/share` by a distribution's voice packages,
/// which nest them by language, region, name and quality.
fn piper_voices() -> &'static [PathBuf] {
    fn walk(dir: &Path, depth: u8, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() && depth > 0 {
                walk(&path, depth - 1, found);
            } else if path.extension().is_some_and(|e| e == "onnx")
                // A model is useless without the settings beside it.
                && path.with_extension("onnx.json").is_file()
            {
                found.push(path);
            }
        }
    }
    static VOICES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    VOICES.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(data) = dirs::data_dir() {
            dirs.push(data.join("piper"));
            dirs.push(data.join("piper-voices"));
        }
        dirs.push("/usr/share/piper".into());
        dirs.push("/usr/share/piper-voices".into());
        let mut found = Vec::new();
        for dir in &dirs {
            walk(dir, 5, &mut found);
        }
        // Sorted, so the same voice is chosen every time.
        found.sort();
        found
    })
}

/// The Piper voice for `lang`, by its file name: `tr_TR-dfki-medium.onnx`.
fn piper_voice(lang: &str) -> Option<&'static Path> {
    piper_voices()
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.split(['_', '-']).next())
                == Some(lang)
        })
        .map(PathBuf::as_path)
}

pub fn can_speak(lang: &str) -> bool {
    speaker().is_some() || (piper().is_some() && player().is_some() && piper_voice(lang).is_some())
}

pub fn command(text: &str, lang: &str) -> Option<Command> {
    if let (Some(piper), Some(player), Some(voice)) = (piper(), player(), piper_voice(lang)) {
        return Some(piper_command(text, piper, player, voice));
    }
    let mut cmd = match speaker()? {
        // `-w` keeps it running until the sentence is over, so the process
        // lasting is what "still speaking" means, the same as the other two.
        Speaker::SpeechDispatcher => {
            let mut c = Command::new("spd-say");
            c.args(["-w", "-l", lang]);
            c
        }
        speaker @ (Speaker::EspeakNg | Speaker::Espeak) => {
            let mut c = Command::new(match speaker {
                Speaker::EspeakNg => "espeak-ng",
                _ => "espeak",
            });
            // eSpeak files Mandarin under its own code rather than "zh".
            c.args(["-v", if lang == "zh" { "cmn" } else { lang }]);
            c
        }
    };
    cmd.arg("--").arg(text);
    Some(cmd)
}

/// Piper writes the whole sentence to a WAV, which is then played. One shell
/// holds both steps so there is a single process to stop: stopping it while
/// Piper is still working means nothing is played, and once playing it has
/// become the player itself (`exec`), so stopping it is stopping the sound.
/// Everything reaches the shell as an argument, never as part of its script.
fn piper_command(text: &str, piper: &Path, player: &str, voice: &Path) -> Command {
    let wav = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("bubbletranslate-speech.wav");
    // Piper reads a line at a time; one line keeps it one recording.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        r#"printf '%s\n' "$1" | "$2" -m "$3" -f "$4" && exec "$5" "$4""#,
        "sh",
        &text,
    ])
    .arg(piper)
    .arg(voice)
    .arg(wav)
    .arg(player);
    cmd
}
