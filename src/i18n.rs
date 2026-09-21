//! The language the app's own interface speaks: English, Turkish or Spanish,
//! switched with one click in the main window, the same three the website has.
//!
//! Held in a global rather than threaded through every draw function: the
//! strings live in the bubble, the main window and a few labels in the config,
//! and all of them only ever need to know "which of three" at the moment they
//! are drawn.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLang {
    #[default]
    En,
    Tr,
    Es,
}

impl UiLang {
    pub const ALL: &'static [UiLang] = &[UiLang::En, UiLang::Tr, UiLang::Es];

    /// The button's face, as the website writes it.
    pub fn code(self) -> &'static str {
        match self {
            UiLang::En => "EN",
            UiLang::Tr => "TR",
            UiLang::Es => "ES",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: UiLang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> UiLang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => UiLang::Tr,
        2 => UiLang::Es,
        _ => UiLang::En,
    }
}

/// Picks the string for the current interface language.
pub fn t(en: &'static str, tr: &'static str, es: &'static str) -> &'static str {
    match lang() {
        UiLang::En => en,
        UiLang::Tr => tr,
        UiLang::Es => es,
    }
}
