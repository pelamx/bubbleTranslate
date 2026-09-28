//! Which program reads text aloud here: the speech synthesiser Windows ships,
//! driven from a hidden PowerShell so that the voice lives in a process of its
//! own and stopping it is ending that process.
//!
//! The voice is picked for the language. When none is installed the script
//! exits with [`crate::speech::NO_VOICE_EXIT`] rather than reading, say,
//! Japanese in an English voice, and the language loses its Listen button.

use std::os::windows::process::CommandExt;
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

/// Keeps PowerShell from opening a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn can_speak(_lang: &str) -> bool {
    true
}

pub fn command(text: &str, lang: &str) -> Option<Command> {
    // Both go into the script's source, so neither may carry anything but
    // what it claims to be: the language is two letters, and the text travels
    // as base64, which also spares it the console's code page.
    if !lang.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let encoded = B64.encode(text);
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $v = $s.GetInstalledVoices() | Where-Object {{ $_.Enabled -and \
              $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq '{lang}' }} | Select-Object -First 1; \
         if (-not $v) {{ exit {no_voice} }}; \
         $s.SelectVoice($v.VoiceInfo.Name); \
         $s.Speak([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')))",
        no_voice = crate::speech::NO_VOICE_EXIT,
    );
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ])
    .creation_flags(CREATE_NO_WINDOW);
    Some(cmd)
}
