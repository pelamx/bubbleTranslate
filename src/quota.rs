//! The free trial, and what does and does not spend it.
//!
//! Ten translations, once, for the life of the install. There is no daily
//! refill: the trial is there to let someone find out whether the bubble is
//! worth two dollars a month, and a wall that dissolves overnight never asks
//! that question.
//!
//! Which is exactly why *what counts* matters more than the number does. The
//! bubble fires on every finished selection, so a user reading a PDF would
//! burn the whole trial in under a minute if every trigger were charged. The
//! rule is deliberately narrow: only a translation that was asked for and
//! actually came back spends anything. Failures, language switches and
//! re-reading the same sentence are all free.
//!
//! What is written to disk is only the count: a total and the flags below. The
//! list of what has already been translated is held in memory and never
//! persisted — an app that reads whatever the user highlights has no business
//! leaving a record of it behind, not even a hashed one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::license::{Entitlement, now};

/// How many recently translated strings stay free to repeat. Enough to cover
/// re-selecting a phrase while reading around it, and — now that the trial
/// does not refill — enough that re-reading what you already paid for with it
/// never costs a second time.
const RECENT_MEMORY: usize = 16;

/// What to do with a selection that has already been captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Translate it, and count it if it succeeds.
    Allow,
    /// Translate it, but do not count it — the same text was translated
    /// before, and reading a sentence twice is one translation.
    Repeat,
    /// The trial is spent. There is no third state: unlike the daily allowance
    /// this replaced, the wall does not come down overnight, so every refusal
    /// carries the way past it.
    Capped { used: u32, limit: u32 },
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => write!(f, "allowed"),
            Self::Repeat => write!(f, "repeat of translated text; free"),
            Self::Capped { used, limit } => write!(f, "{used}/{limit} used; trial spent"),
        }
    }
}

/// The persisted half. Nothing here is derived from anything the user
/// selected, which is what keeps this file uninteresting if it is ever read.
#[derive(Debug, Serialize, Deserialize)]
struct Counter {
    /// Translations spent from the trial, for the life of this install.
    ///
    /// There is no `day` beside it any more and no high-water mark: a counter
    /// that never resets has nothing to protect against a clock moved
    /// backwards, which removes the whole class of bug that logic existed for.
    #[serde(default)]
    used: u32,
    /// When this install first ran a metered build.
    #[serde(default)]
    first_seen: u64,
    /// Set once, on an install that predates metering: it keeps unlimited use
    /// forever. Taking something away from people who already have it is the
    /// one thing this feature cannot undo, so it does not try.
    #[serde(default)]
    legacy_unlimited: bool,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            used: 0,
            first_seen: now(),
            legacy_unlimited: false,
        }
    }
}

pub struct Quota {
    counter: Counter,
    /// Hashes of what has been translated in this session. In memory only, on
    /// purpose.
    recent: Vec<u64>,
}

impl Quota {
    /// Loads the counter, creating it on first run.
    ///
    /// `config_existed` answers the one question that can only be asked once:
    /// whether this machine was already running bubbleTranslate before metering
    /// existed. It has to be sampled before [`crate::config::Config::load`],
    /// which writes the file it is asking about.
    ///
    /// A counter written by a build that metered per day is read here without
    /// ceremony: `day` and `high_water_day` are simply not fields any more, and
    /// `used` carries over. Someone who had spent three of that day's five
    /// arrives with three of ten spent, which is the generous reading and the
    /// only one that does not punish an existing user for upgrading.
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
        // First run under a build that has this field: date the install now
        // rather than leaving it at zero, so it means something later.
        if quota.counter.first_seen == 0 {
            quota.counter.first_seen = now();
            quota.save();
        }
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

    /// Translations in total, or `None` for unlimited.
    pub fn limit(&self, entitlement: &Entitlement) -> Option<u32> {
        if self.counter.legacy_unlimited {
            return None;
        }
        entitlement.limit
    }

    pub fn used(&self) -> u32 {
        self.counter.used
    }

    /// What is left of the trial, or `None` on an unlimited install.
    pub fn remaining(&self, entitlement: &Entitlement) -> Option<u32> {
        self.limit(entitlement)
            .map(|limit| limit.saturating_sub(self.counter.used))
    }

    pub fn is_grandfathered(&self) -> bool {
        self.counter.legacy_unlimited
    }

    /// Whether this selection may be translated, and whether it will be charged.
    ///
    /// Takes `&self`: deciding costs nothing and records nothing. Only
    /// [`Self::record`] moves the counter, and only for a translation that
    /// actually came back.
    pub fn verdict(&self, text: &str, entitlement: &Entitlement) -> Verdict {
        let Some(limit) = self.limit(entitlement) else {
            return Verdict::Allow;
        };
        if self.recent.contains(&digest(text)) {
            return Verdict::Repeat;
        }
        if self.counter.used < limit {
            return Verdict::Allow;
        }
        Verdict::Capped {
            used: self.counter.used,
            limit,
        }
    }

    /// Records a translation that actually came back. Called only on success,
    /// and only for a [`Verdict::Allow`].
    pub fn record(&mut self, text: &str) {
        self.counter.used = self.counter.used.saturating_add(1);
        self.recent.push(digest(text));
        if self.recent.len() > RECENT_MEMORY {
            self.recent.remove(0);
        }
        self.save();
    }

    /// Hands the trial back, which is what activating a licence and then
    /// removing it must not silently do — see the test. Only ever called from
    /// the development helper.
    #[cfg(debug_assertions)]
    pub fn reset_trial(&mut self) {
        self.counter.used = 0;
        self.recent.clear();
        self.save();
    }

    fn save(&self) {
        // Tests build counters directly and must never reach the real file —
        // running `cargo test` should not spend, reset, or resurrect anybody's
        // trial.
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

fn digest(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::license::{FREE_TRIAL_TRANSLATIONS, Plan};

    fn quota(used: u32) -> Quota {
        Quota {
            counter: Counter {
                used,
                first_seen: 0,
                legacy_unlimited: false,
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
            cycle: None,
            limit: None,
            exp: u64::MAX,
        }
    }

    /// The trial runs out, and says so with the numbers the bubble shows.
    #[test]
    fn the_tenth_translation_is_the_last_free_one() {
        let mut q = quota(0);
        for n in 0..FREE_TRIAL_TRANSLATIONS {
            assert_eq!(q.verdict(&format!("text {n}"), &free()), Verdict::Allow);
            q.record(&format!("text {n}"));
        }
        assert!(matches!(
            q.verdict("something new", &free()),
            Verdict::Capped { used: 10, limit: 10, .. },
        ));
    }

    /// The trial does not come back tomorrow. This is the whole difference
    /// between this file and the daily allowance it replaced, and with the
    /// upgrade throttle gone it is now literally true of the code: nothing in
    /// this file reads a clock at all, so there is no moment at which it could
    /// decide to be generous again.
    #[test]
    fn a_spent_trial_stays_spent() {
        let q = quota(FREE_TRIAL_TRANSLATIONS);
        assert!(matches!(q.verdict("one", &free()), Verdict::Capped { .. }));

        // Reload the way a launch tomorrow would, from what was written.
        let json = serde_json::to_string(&q.counter).unwrap();
        let counter: Counter = serde_json::from_str(&json).unwrap();
        let next_launch = Quota {
            counter,
            recent: Vec::new(),
        };
        assert_eq!(next_launch.used(), FREE_TRIAL_TRANSLATIONS);
        assert!(matches!(
            next_launch.verdict("two", &free()),
            Verdict::Capped { .. },
        ));
    }

    /// A counter written by a build that metered per day still loads, and its
    /// day fields are simply gone rather than fatal.
    #[test]
    fn a_daily_counter_from_an_older_build_carries_its_count_over() {
        let old = r#"{"day":20699,"used":3,"high_water_day":20699,
                      "first_seen":1750000000,"legacy_unlimited":false,
                      "prompted_at":0}"#;
        let counter: Counter = serde_json::from_str(old).expect("an old counter still parses");
        assert_eq!(counter.used, 3);
        assert!(!counter.legacy_unlimited);

        // And the grandfathering flag survives, which matters far more: it is
        // the one thing in this file that cannot be reconstructed if lost.
        let legacy = r#"{"day":1,"used":99,"high_water_day":1,"legacy_unlimited":true}"#;
        let counter: Counter = serde_json::from_str(legacy).unwrap();
        assert!(counter.legacy_unlimited);
    }

    /// Re-reading a sentence is one translation, not two. Without this, the
    /// language picker and an idle re-selection both cost the user trial.
    #[test]
    fn re_selecting_the_same_text_is_free() {
        let mut q = quota(0);
        q.record("merhaba dünya");
        // Spend the rest of the trial on other things.
        for n in 0..(FREE_TRIAL_TRANSLATIONS - 1) {
            q.record(&format!("other {n}"));
        }
        assert_eq!(q.used(), FREE_TRIAL_TRANSLATIONS);
        assert_eq!(q.verdict("merhaba dünya", &free()), Verdict::Repeat);
        // Whitespace differences are the same selection to a human.
        assert_eq!(q.verdict("  merhaba dünya  ", &free()), Verdict::Repeat);
        assert!(matches!(
            q.verdict("a different sentence", &free()),
            Verdict::Capped { .. },
        ));
    }

    /// Every refusal carries the way past it.
    ///
    /// The allowance this replaced showed the upgrade button once an hour, so
    /// as not to nag someone whose allowance would refill at midnight. A trial
    /// has no midnight: buying is the only way forward, and a refusal that
    /// hides it answers the gesture with a dead end.
    #[test]
    fn every_refusal_offers_the_way_out() {
        let q = quota(FREE_TRIAL_TRANSLATIONS);
        for attempt in ["one", "two", "three"] {
            assert!(
                matches!(q.verdict(attempt, &free()), Verdict::Capped { .. }),
                "{attempt} was not refused",
            );
        }
    }

    /// What is left is what the account panel and the trial bubble both count
    /// down, so an over-spent counter must not underflow into a huge number.
    #[test]
    fn remaining_never_underflows() {
        assert_eq!(quota(0).remaining(&free()), Some(FREE_TRIAL_TRANSLATIONS));
        assert_eq!(quota(4).remaining(&free()), Some(6));
        assert_eq!(quota(FREE_TRIAL_TRANSLATIONS).remaining(&free()), Some(0));
        assert_eq!(quota(9_000).remaining(&free()), Some(0));
        assert_eq!(quota(0).remaining(&pro()), None);
    }

    /// Pro is unlimited, and an install that predates metering stays unlimited
    /// whatever the entitlement says.
    #[test]
    fn unlimited_means_unlimited() {
        let q = quota(9_000);
        assert_eq!(q.verdict("anything", &pro()), Verdict::Allow);

        let mut legacy = quota(9_000);
        legacy.counter.legacy_unlimited = true;
        assert_eq!(legacy.limit(&free()), None);
        assert_eq!(legacy.verdict("anything", &free()), Verdict::Allow);
    }
}
