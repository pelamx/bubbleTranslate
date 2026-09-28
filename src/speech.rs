//! Reading a translation aloud.
//!
//! Every system already has a voice, so none is carried here: each platform
//! says which program speaks and how (see `platform::<os>::speech`), and this
//! side owns the one process that is speaking, so a second press, a new
//! translation or the bubble going away can stop it.

use std::process::{Child, Stdio};
use std::sync::Mutex;

use crate::platform::speech_command;

/// What a platform's speaking program exits with when it has no voice for the
/// language it was asked for. The language is then left without a button, so
/// the next bubble does not offer something it cannot do.
pub const NO_VOICE_EXIT: i32 = 3;

/// Long enough for any bubble; short enough to stay inside the Windows
/// command-line limit once the text is encoded onto it.
const MAX_CHARS: usize = 4000;

/// The process reading right now, and the language it was asked to read.
static CURRENT: Mutex<Option<(Child, String)>> = Mutex::new(None);
static NO_VOICE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn base(lang: &str) -> &str {
    lang.split('-').next().unwrap_or(lang)
}

/// Whether there is a voice that can read `lang`.
pub fn available(lang: &str) -> bool {
    let lang = base(lang);
    !NO_VOICE.lock().unwrap().iter().any(|l| l == lang) && speech_command::can_speak(lang)
}

/// Starts reading `text` in `lang`, stopping whatever was being read.
pub fn speak(text: &str, lang: &str) {
    stop();
    let lang = base(lang);
    let text: String = text.chars().take(MAX_CHARS).collect();
    let Some(mut cmd) = speech_command::command(&text, lang) else {
        return;
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.spawn() {
        Ok(child) => *CURRENT.lock().unwrap() = Some((child, lang.to_string())),
        Err(e) => crate::trace!("speech    could not start: {e}"),
    }
}

/// Whether something is being read right now. Also reaps the process once it
/// has finished, which is what turns "Stop" back into "Listen".
pub fn speaking() -> bool {
    let mut current = CURRENT.lock().unwrap();
    let Some((child, lang)) = current.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => return true,
        Ok(Some(status)) if status.code() == Some(NO_VOICE_EXIT) => {
            crate::trace!("speech    no voice for {lang}");
            NO_VOICE.lock().unwrap().push(std::mem::take(lang));
        }
        _ => {}
    }
    *current = None;
    false
}

/// Stops reading, if anything is.
pub fn stop() {
    if let Some((mut child, _)) = CURRENT.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
