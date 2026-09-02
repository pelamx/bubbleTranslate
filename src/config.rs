//! User settings, persisted as TOML in the platform config dir.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which backend to ask for a translation. The engine walks `Config::providers`
/// in order and keeps the first answer it gets, so ordering here is the
/// failover policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    MyMemory,
    DeepL,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::MyMemory => "MyMemory",
            Provider::DeepL => "DeepL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Language to translate into, as an ISO-639-1 code ("en", "tr", "de", ...).
    pub target_lang: String,
    /// Source language, or "auto" to let the provider detect it.
    pub source_lang: String,
    /// Failover order. Google first by default: no key, no quota to register.
    pub providers: Vec<Provider>,
    /// DeepL API key. Free keys end in ":fx"; the endpoint is chosen from that
    /// suffix. Without a key DeepL is skipped even if listed in `providers`.
    pub deepl_api_key: String,
    /// Optional contact address for MyMemory. Anonymous use is capped at ~5k
    /// chars/day; supplying an address raises it to ~50k.
    pub mymemory_email: String,
    /// Pop the bubble automatically when a selection is made.
    pub auto_translate: bool,
    /// Selections shorter/longer than these bounds are ignored. The upper bound
    /// keeps a stray Cmd+A out of the translation queue.
    pub min_chars: usize,
    pub max_chars: usize,
    /// How long the selection has to hold still before it counts as finished.
    ///
    /// The window slides, so this is a quiet period rather than a delay: a
    /// selection that is still growing keeps restarting it, and the bubble
    /// waits for the sweep to end instead of appearing partway through it.
    pub debounce_ms: u64,
    /// Allow synthesizing Cmd+C when the Accessibility API returns nothing.
    pub clipboard_fallback: bool,
    /// Also translate whatever gets copied, not only what gets selected.
    ///
    /// Selecting text publishes it to the desktop by itself, which is what
    /// makes the bubble work without any cooperation from the application
    /// being read. A few applications never publish a selection — anything
    /// drawing its own text, this app's own window included — and for those,
    /// copying is the one gesture that always reaches the desktop. Off by
    /// default, because with it on every copy pops a bubble.
    pub watch_clipboard: bool,
    /// Start with no interface: no main window, just the bubble and whatever
    /// indicator the desktop gives us.
    ///
    /// What a translator is for most of the time — it watches selections and
    /// stays out of the way, and the settings window is somewhere you visit,
    /// not somewhere you live. Off by default so a first run shows the app
    /// exists. Ignored where nothing can bring the window back: starting
    /// invisible with no indicator would be starting unreachable.
    pub start_in_background: bool,
    /// Seconds of no interaction before the bubble hides itself. 0 keeps it up
    /// until it is closed or replaced. The countdown pauses while the pointer
    /// is over the bubble.
    pub auto_hide_secs: u64,
    pub font_size: f32,
    /// Scales the whole interface, on top of whatever the display's own
    /// scaling works out to.
    ///
    /// Exists because there is no way to ask a desktop how large its text is.
    /// The display's scale can be read and matched — and is, see
    /// `platform::preferred_zoom` — but that only settles what a point is
    /// worth in pixels, not how big a desktop's own applications choose to
    /// draw. Some run denser than others, and this is the dial for it.
    pub ui_scale: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_lang: "en".to_string(),
            source_lang: "auto".to_string(),
            providers: vec![Provider::Google, Provider::MyMemory, Provider::DeepL],
            deepl_api_key: String::new(),
            mymemory_email: String::new(),
            auto_translate: true,
            min_chars: 2,
            max_chars: 4000,
            debounce_ms: 180,
            clipboard_fallback: true,
            watch_clipboard: false,
            start_in_background: false,
            auto_hide_secs: 12,
            font_size: 16.0,
            // The type scale here was drawn against macOS, whose system
            // interface runs looser than most Linux desktops do; matching the
            // display's scaling alone still leaves the app noticeably larger
            // than its neighbours. This is a starting point, not a verdict —
            // the slider in the main window is the real answer.
            ui_scale: if cfg!(target_os = "linux") { 0.85 } else { 1.0 },
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bubbleTranslate")
            .join("config.toml")
    }

    /// Reads the config, falling back to defaults when it is missing or
    /// unparseable. A broken config should never stop the app from starting.
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            let cfg = Config::default();
            let _ = cfg.save();
            return cfg;
        };
        match toml::from_str(&raw) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!(
                    "bubbleTranslate: {} is invalid ({err}); using defaults",
                    path.display()
                );
                Config::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Providers that can actually run right now, in failover order. DeepL
    /// drops out when unconfigured so it never costs a round trip.
    pub fn active_providers(&self) -> Vec<Provider> {
        self.providers
            .iter()
            .copied()
            .filter(|p| *p != Provider::DeepL || !self.deepl_api_key.trim().is_empty())
            .collect()
    }
}

/// Languages offered in the bubble's picker. Kept short on purpose; any code
/// the providers accept can be typed into the config file directly.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("tr", "Türkçe"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("ar", "العربية"),
    ("fa", "فارسی"),
    ("hi", "हिन्दी"),
    ("zh", "中文"),
    ("ja", "日本語"),
    ("ko", "한국어"),
];

pub fn language_name(code: &str) -> &str {
    let base = code.split('-').next().unwrap_or(code);
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == base)
        .map(|(_, name)| *name)
        .unwrap_or(code)
}
