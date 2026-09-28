//! Which program reads text aloud here.
//!
//! Nothing is bundled — the release stays a single binary — so this uses what
//! the desktop already has. Speech Dispatcher first, because it is what the
//! desktop's own screen reader talks to and it speaks with whichever voice the
//! user set up; eSpeak on its own after that. With neither installed the
//! bubble simply has no Listen button.

use std::path::PathBuf;
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

pub fn can_speak(_lang: &str) -> bool {
    speaker().is_some()
}

pub fn command(text: &str, lang: &str) -> Option<Command> {
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
