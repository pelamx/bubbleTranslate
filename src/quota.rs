//! The daily allowance, and what does and does not spend it.
//!
//! Five a day is a small number, so *what counts* matters more than the number
//! does. The bubble fires on every finished selection, which means a user
//! reading a PDF would burn the day's allowance in under a minute if every
//! trigger were charged. So the rule is deliberately narrow: only a
//! translation that was asked for and actually came back spends anything.
//! Failures, language switches and re-reading the same sentence are all free.
//!
//! What is written to disk is only the count: a day number, a total, and the
//! flags below. The list of what has already been translated today is held in
//! memory and never persisted — an app that reads whatever the user highlights
//! has no business leaving a record of it behind, not even a hashed one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::license::{Entitlement, now};

/// How many recently translated strings stay free to repeat. Enough to cover
/// re-selecting a phrase while reading around it; small enough that the whole
/// day's work is not exempt.
const RECENT_MEMORY: usize = 16;

/// How long after showing the upgrade bubble before it may appear again.
///
/// Once per day would be tidier, but an app that stops responding to a gesture
/// for the rest of the day is indistinguishable from a broken one, and gets
/// reported as one. Once an hour keeps the reminder honest without turning the
/// bubble into an advertisement.
const REPROMPT_SECS: u64 = 3600;

/// What to do with a selection that has already been captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Translate it, and count it if it succeeds.
    Allow,
    /// Translate it, but do not count it — the same text was translated
    /// earlier today, and reading a sentence twice is one translation.
    Repeat,
    /// Over the allowance. `prompt` is false when the upgrade bubble was shown
    /// recently enough that showing it again would be nagging.
    Capped { used: u32, limit: u32, prompt: bool },
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => write!(f, "allowed"),
            Self::Repeat => write!(f, "repeat of today's text; free"),
            Self::Capped { used, limit, prompt } => {
                write!(f, "{used}/{limit} used; capped (prompt={prompt})")
            }
        }
    }
}

/// The persisted half. Nothing here is derived from anything the user
/// selected, which is what keeps this file uninteresting if it is ever read.
#[derive(Debug, Serialize, Deserialize)]
struct Counter {
    /// Days since the epoch, in local time — see [`local_day`].
    day: i64,
    used: u32,
    /// The furthest-forward day ever seen. A `day` behind this one means the
    /// clock moved, not that time passed.
    high_water_day: i64,
    /// When this install first ran a metered build. Kept so a first-week
    /// runway can be added later without needing to have been there all along.
    #[serde(default)]
    first_seen: u64,
    /// Set once, on an install that predates metering: it keeps unlimited use
    /// forever. Taking something away from people who already have it is the
    /// one thing this feature cannot undo, so it does not try.
    #[serde(default)]
    legacy_unlimited: bool,
    /// Unix seconds of the last upgrade prompt.
    #[serde(default)]
    prompted_at: u64,
}

impl Default for Counter {
    fn default() -> Self {
        let today = local_day();
        Self {
            day: today,
            used: 0,
            high_water_day: today,
            first_seen: now(),
            legacy_unlimited: false,
            prompted_at: 0,
        }
    }
}

pub struct Quota {
    counter: Counter,
    /// Hashes of what has been translated today. In memory only, on purpose.
    recent: Vec<u64>,
}

impl Quota {
    /// Loads the counter, creating it on first run.
    ///
    /// `config_existed` answers the one question that can only be asked once:
    /// whether this machine was already running bubbleTranslate before metering
    /// existed. It has to be sampled before [`crate::config::Config::load`],
    /// which writes the file it is asking about.
    pub fn load(config_existed: bool) -> Self {
        let mut quota = match std::fs::read_to_string(path()).ok() {
            Some(raw) => match serde_json::from_str::<Counter>(&raw) {
                Ok(counter) => Self {
                    counter,
                    recent: Vec::new(),
                },
                Err(err) => {
                    // A broken counter must never stop the app from starting,
                    // and must never be read as "unlimited" either.
                    eprintln!(
                        "bubbleTranslate: {} is unreadable ({err}); starting a new count",
                        path().display(),
                    );
                    Self::fresh(false)
                }
            },
            // No counter file. Either a genuinely new install, or an existing
            // one meeting a metered build for the first time — and the config
            // file is what tells the two apart.
            None => {
                let quota = Self::fresh(config_existed);
                if config_existed {
                    crate::trace!("quota     existing install; unlimited use preserved");
                }
                quota.save();
                quota
            }
        };
        quota.roll();
        quota
    }

    fn fresh(legacy_unlimited: bool) -> Self {
        Self {
            counter: Counter {
                legacy_unlimited,
                ..Counter::default()
            },
            recent: Vec::new(),
        }
    }

    /// Translations a day, or `None` for unlimited.
    pub fn limit(&self, entitlement: &Entitlement) -> Option<u32> {
        if self.counter.legacy_unlimited {
            return None;
        }
        entitlement.daily_limit
    }

    pub fn used_today(&self) -> u32 {
        self.counter.used
    }

    pub fn is_grandfathered(&self) -> bool {
        self.counter.legacy_unlimited
    }

    /// Whether this selection may be translated, and whether it will be charged.
    pub fn verdict(&mut self, text: &str, entitlement: &Entitlement) -> Verdict {
        self.roll();

        let Some(limit) = self.limit(entitlement) else {
            return Verdict::Allow;
        };
        if self.recent.contains(&digest(text)) {
            return Verdict::Repeat;
        }
        if self.counter.used < limit {
            return Verdict::Allow;
        }

        let now = now();
        let prompt = now.saturating_sub(self.counter.prompted_at) >= REPROMPT_SECS;
        if prompt {
            self.counter.prompted_at = now;
            self.save();
        }
        Verdict::Capped {
            used: self.counter.used,
            limit,
            prompt,
        }
    }

    /// Records a translation that actually came back. Called only on success,
    /// and only for a [`Verdict::Allow`].
    pub fn record(&mut self, text: &str) {
        self.roll();
        self.counter.used = self.counter.used.saturating_add(1);
        self.recent.push(digest(text));
        if self.recent.len() > RECENT_MEMORY {
            self.recent.remove(0);
        }
        self.save();
    }

    /// Moves the counter to today, if today is later than the day it is on.
    ///
    /// A day number *behind* the high-water mark means the system clock went
    /// backwards — a timezone change, an NTP correction, or someone hoping for
    /// a fresh five. The count is kept in that case, and it is kept again when
    /// the clock catches back up, because the high-water mark is what the
    /// comparison is against rather than the current day.
    fn roll(&mut self) {
        let today = local_day();
        if today == self.counter.day {
            return;
        }
        if today > self.counter.high_water_day {
            self.counter.used = 0;
            self.counter.prompted_at = 0;
            self.counter.high_water_day = today;
            self.recent.clear();
        }
        self.counter.day = today;
        self.save();
    }

    fn save(&self) {
        // Tests build counters directly and must never reach the real file —
        // running `cargo test` should not spend, reset, or resurrect anybody's
        // allowance.
        if cfg!(test) {
            return;
        }
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.counter) {
            // A failed write costs the user nothing worse than a count that
            // restarts next launch, so it is not worth interrupting them over.
            if let Err(err) = std::fs::write(&path, json) {
                crate::trace!("quota     could not save {}: {err}", path.display());
            }
        }
    }
}

/// The counter lives beside the licence, in the data directory rather than in
/// the config directory: `config.toml` is a file users are told to edit, and
/// their target language should not share a file with their meter.
pub fn path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bubbleTranslate")
        .join("usage.json")
}

/// Days since the epoch in local time, so the allowance resets at the user's
/// midnight rather than at UTC's.
///
/// Stored as a number rather than a date string so that comparing two of them
/// is unambiguous no matter how the date would have been formatted.
fn local_day() -> i64 {
    let unix = now() as i64;
    (unix + local_offset(unix)) / 86_400
}

/// Seconds east of UTC at the given moment, which is what makes this correct
/// across a daylight-saving change rather than only at the moment it is read.
fn local_offset(unix: i64) -> i64 {
    // SAFETY: `localtime_r` fills a `tm` the caller owns and is the reentrant
    // form of `localtime`, so nothing here reads or writes shared state. A
    // null return means the conversion failed, and UTC is the honest fallback.
    unsafe {
        let time = unix as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
}

fn digest(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::license::Plan;

    fn quota(used: u32, day: i64, high_water: i64) -> Quota {
        Quota {
            counter: Counter {
                day,
                used,
                high_water_day: high_water,
                first_seen: 0,
                legacy_unlimited: false,
                prompted_at: 0,
            },
            recent: Vec::new(),
        }
    }

    fn free() -> Entitlement {
        Entitlement::free()
    }

    fn pro() -> Entitlement {
        Entitlement {
            plan: Plan::Pro,
            daily_limit: None,
            exp: u64::MAX,
        }
    }

    /// The allowance runs out, and says so with the numbers the bubble shows.
    #[test]
    fn the_fifth_translation_is_the_last_free_one() {
        let mut q = quota(0, local_day(), local_day());
        for n in 0..5 {
            assert_eq!(q.verdict(&format!("text {n}"), &free()), Verdict::Allow);
            q.record(&format!("text {n}"));
        }
        assert!(matches!(
            q.verdict("something new", &free()),
            Verdict::Capped { used: 5, limit: 5, .. },
        ));
    }

    /// Re-reading a sentence is one translation, not two. Without this, the
    /// language picker and an idle re-selection both cost the user quota.
    #[test]
    fn re_selecting_the_same_text_is_free() {
        let mut q = quota(0, local_day(), local_day());
        q.record("merhaba dünya");
        // Spend the rest of the allowance on other things.
        for n in 0..4 {
            q.record(&format!("other {n}"));
        }
        assert_eq!(q.used_today(), 5);
        assert_eq!(q.verdict("merhaba dünya", &free()), Verdict::Repeat);
        // Whitespace differences are the same selection to a human.
        assert_eq!(q.verdict("  merhaba dünya  ", &free()), Verdict::Repeat);
        assert!(matches!(
            q.verdict("a different sentence", &free()),
            Verdict::Capped { .. },
        ));
    }

    /// The prompt appears when the wall is first hit and then stays quiet,
    /// rather than answering every selection with an advertisement.
    #[test]
    fn the_upgrade_prompt_does_not_repeat_immediately() {
        let mut q = quota(5, local_day(), local_day());
        assert!(matches!(
            q.verdict("one", &free()),
            Verdict::Capped { prompt: true, .. },
        ));
        assert!(matches!(
            q.verdict("two", &free()),
            Verdict::Capped { prompt: false, .. },
        ));
    }

    /// A new day restores the allowance.
    #[test]
    fn tomorrow_starts_over() {
        let today = local_day();
        let mut q = quota(5, today - 1, today - 1);
        assert_eq!(q.verdict("anything", &free()), Verdict::Allow);
        assert_eq!(q.used_today(), 0);
    }

    /// Winding the clock back does not hand out a second allowance — and the
    /// count survives the clock catching up again.
    #[test]
    fn a_clock_moved_backwards_does_not_refill_the_allowance() {
        let today = local_day();
        let mut q = quota(5, today, today);

        // Yesterday, according to a clock that has been tampered with.
        q.counter.day = today - 3;
        q.roll();
        assert_eq!(q.used_today(), 5, "winding back refilled the allowance");

        // And back to the real date, which is still not a new day.
        q.counter.day = today - 3;
        q.counter.high_water_day = today;
        q.roll();
        assert_eq!(q.used_today(), 5);
    }

    /// Pro is unlimited, and an install that predates metering stays unlimited
    /// whatever the entitlement says.
    #[test]
    fn unlimited_means_unlimited() {
        let mut q = quota(9_000, local_day(), local_day());
        assert_eq!(q.verdict("anything", &pro()), Verdict::Allow);

        let mut legacy = quota(9_000, local_day(), local_day());
        legacy.counter.legacy_unlimited = true;
        assert_eq!(legacy.limit(&free()), None);
        assert_eq!(legacy.verdict("anything", &free()), Verdict::Allow);
    }
}
