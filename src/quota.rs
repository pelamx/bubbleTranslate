//! The daily allowance, and what does and does not spend it.
//!
//! Ten translations a day on the free tier. The count comes back at the
//! user's midnight, so someone who runs out is never more than a night away
//! from the free path — and Pro is there for anyone who does not want to wait.
//!
//! Ten is a small number, so *what counts* matters more than the number does.
//! The bubble fires on every finished selection, which means a user reading a
//! PDF would burn the day's allowance in under a minute if every trigger were
//! charged. So the rule is deliberately narrow: only a translation that was
//! asked for and actually came back spends anything. Failures, language
//! switches and re-reading the same sentence are all free.
//!
//! What is written to disk is only the count: a day number, a total, and the
//! flags below. The list of what has already been translated today is held in
//! memory and never persisted — an app that reads whatever the user highlights
//! has no business leaving a record of it behind, not even a hashed one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::license::{Entitlement, FREE_DAILY_TRANSLATIONS, now};

/// How many recently translated strings stay free to repeat. Enough to cover
/// re-selecting a phrase while reading around it; small enough that the whole
/// day's work is not exempt.
const RECENT_MEMORY: usize = 16;

/// Mixed into the counter's seal. Not a secret -- it is in the binary -- but
/// it keeps the seal from being a plain hash anyone could recompute by guess.
const SEAL_SALT: &str = "bubbleTranslate.usage.v1";

/// What to do with a selection that has already been captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Translate it, and count it if it succeeds.
    Allow,
    /// Translate it, but do not count it — the same text was translated
    /// earlier today, and reading a sentence twice is one translation.
    Repeat,
    /// Today's allowance is spent. Every refusal carries the way past it: the
    /// bubble says so and offers Pro, rather than going quiet until midnight.
    Capped { used: u32, limit: u32 },
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => write!(f, "allowed"),
            Self::Repeat => write!(f, "repeat of today's text; free"),
            Self::Capped { used, limit } => write!(f, "{used}/{limit} used today; capped"),
        }
    }
}

/// The persisted half. Nothing here is derived from anything the user
/// selected, which is what keeps this file uninteresting if it is ever read.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    /// Days since the epoch, in local time — see [`local_day`]. Missing from
    /// a counter written by the build that metered once per install; a zero
    /// here is simply an old day, and the count rolls over on first read.
    #[serde(default)]
    day: i64,
    /// Translations spent today.
    #[serde(default)]
    used: u32,
    /// The furthest-forward day ever seen. A `day` behind this one means the
    /// clock moved, not that time passed.
    #[serde(default)]
    high_water_day: i64,
    /// When this install first ran a metered build.
    #[serde(default)]
    first_seen: u64,
    /// A hash of the fields above and this machine's id. A counter edited by
    /// hand, copied from another machine or written by an older build does
    /// not match it, and is read as a day already spent -- see [`Quota::load`].
    #[serde(default)]
    seal: String,
}

impl Default for Counter {
    fn default() -> Self {
        let today = local_day();
        Self {
            day: today,
            used: 0,
            high_water_day: today,
            first_seen: now(),
            seal: String::new(),
        }
    }
}

pub struct Quota {
    counter: Counter,
    /// Hashes of what has been translated today. In memory only, on purpose.
    recent: Vec<u64>,
    /// This machine's id, read once: the seal is recomputed on every save, and
    /// on macOS reading the id means running a program.
    machine: String,
}

impl Quota {
    /// Loads the counter, creating it on first run.
    ///
    /// The allowance lives on this machine, so the file is the thing someone
    /// wanting more of it would edit or delete. Neither is allowed to pay:
    ///
    /// - A counter whose seal does not match -- edited, or copied from another
    ///   machine -- is kept, but today is counted as spent. Tomorrow starts
    ///   over as usual.
    /// - No counter at all, on a machine that already has a config, is a
    ///   deleted counter rather than a new install, and is treated the same.
    /// - An unreadable counter, likewise.
    ///
    /// Only a machine with neither file is a first run with ten in hand.
    /// `config_existed` has to be sampled before
    /// [`crate::config::Config::load`], which writes the file it asks about.
    ///
    /// An honest user pays for this at most on the day their machine id
    /// changes. The legacy flag older builds wrote is ignored: only Pro is
    /// unlimited.
    pub fn load(config_existed: bool) -> Self {
        let machine = crate::license::device_id();
        let raw = std::fs::read_to_string(path()).ok();
        let mut quota = Self::open(raw.as_deref(), config_existed, machine);
        quota.save();
        quota.roll();
        quota
    }

    /// The decision in [`Self::load`], without the disk.
    fn open(raw: Option<&str>, config_existed: bool, machine: String) -> Self {
        let (counter, trusted) = match raw {
            Some(raw) => match serde_json::from_str::<Counter>(raw) {
                // No seal at all is a counter from before the seal, taken as
                // it is once so an upgrade costs nobody a day. Stripping the
                // seal to get the same treatment buys no more than deleting
                // both files does, which no local check can prevent.
                Ok(counter) => {
                    let trusted =
                        counter.seal.is_empty() || counter.seal == seal(&counter, &machine);
                    (counter, trusted)
                }
                Err(err) => {
                    eprintln!(
                        "bubbleTranslate: {} is unreadable ({err}); today counts as spent",
                        path().display(),
                    );
                    (Counter::default(), false)
                }
            },
            None => (Counter::default(), !config_existed),
        };
        let mut quota = Self {
            counter,
            recent: Vec::new(),
            machine,
        };
        if quota.counter.first_seen == 0 {
            quota.counter.first_seen = now();
        }
        if !trusted {
            crate::trace!("quota     counter missing or not sealed here; today counts as spent");
            quota.spend_today();
        }
        quota
    }

    /// Marks today's allowance as used up, without touching any other day.
    fn spend_today(&mut self) {
        let today = local_day();
        self.counter.day = today;
        self.counter.high_water_day = self.counter.high_water_day.max(today);
        self.counter.used = self.counter.used.max(FREE_DAILY_TRANSLATIONS);
        self.recent.clear();
    }

    /// Translations a day, or `None` for unlimited.
    pub fn limit(&self, entitlement: &Entitlement) -> Option<u32> {
        entitlement.limit
    }

    /// Translations spent today. `&mut` because asking rolls the day over
    /// first: a count read across midnight would otherwise be yesterday's.
    pub fn used_today(&mut self) -> u32 {
        self.roll();
        self.counter.used
    }

    /// What is left of today's allowance, or `None` on an unlimited install.
    pub fn remaining(&mut self, entitlement: &Entitlement) -> Option<u32> {
        self.roll();
        self.limit(entitlement)
            .map(|limit| limit.saturating_sub(self.counter.used))
    }

    /// Whether this selection may be translated, and whether it will be charged.
    ///
    /// Deciding records nothing beyond the day rolling over. Only
    /// [`Self::record`] moves the count, and only for a translation that
    /// actually came back.
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
        Verdict::Capped {
            used: self.counter.used,
            limit,
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
    /// a fresh ten. The count is kept in that case, and it is kept again when
    /// the clock catches back up, because the high-water mark is what the
    /// comparison is against rather than the current day.
    ///
    /// The forward direction is not guarded the same way, on purpose: a clock
    /// genuinely set ahead deserves the fresh day it asks for, and telling a
    /// corrected clock from an advancing one needs an outside opinion this
    /// offline counter does not have. The file itself is the easier edit
    /// anyway — the meter is a nudge toward Pro, not a wall.
    fn roll(&mut self) {
        let today = local_day();
        if today == self.counter.day {
            return;
        }
        if today > self.counter.high_water_day {
            self.counter.used = 0;
            self.counter.high_water_day = today;
            self.recent.clear();
        }
        self.counter.day = today;
        self.save();
    }

    /// Hands today's allowance back without waiting for midnight. Only ever
    /// called from the development helper.
    #[cfg(debug_assertions)]
    pub fn reset_today(&mut self) {
        self.counter.used = 0;
        self.recent.clear();
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
        let counter = Counter {
            seal: seal(&self.counter, &self.machine),
            ..self.counter.clone()
        };
        if let Ok(json) = serde_json::to_string_pretty(&counter) {
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
    if let Some(home) = crate::config::state_home() {
        return home.join("usage.json");
    }
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
pub(crate) fn local_day() -> i64 {
    let unix = now() as i64;
    (unix + local_offset(unix)) / 86_400
}

/// Seconds east of UTC at the given moment, which is what makes this correct
/// across a daylight-saving change rather than only at the moment it is read.
#[cfg(unix)]
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

/// The same question, asked the way Windows answers it: convert the moment to
/// a wall clock in the user's own time zone and take the difference.
///
/// Asked *about a moment* rather than about now, for the same reason as the
/// Unix side: a counter written before the clocks changed must still be read
/// as the day it was written on.
#[cfg(target_os = "windows")]
fn local_offset(unix: i64) -> i64 {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };

    /// Seconds between 1601-01-01, where Windows starts counting, and the
    /// Unix epoch.
    const EPOCH_DIFFERENCE: i64 = 11_644_473_600;

    let ticks = (unix + EPOCH_DIFFERENCE) * 10_000_000;
    if ticks < 0 {
        return 0;
    }
    let utc = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };

    // SAFETY: every call below fills a structure this function owns, and each
    // is checked before its result is used. A failure leaves UTC, which is the
    // honest answer when the system will not say what zone it is in.
    unsafe {
        let mut universal = SYSTEMTIME::default();
        if FileTimeToSystemTime(&utc, &mut universal).is_err() {
            return 0;
        }
        let mut local = SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &universal, &mut local).is_err() {
            return 0;
        }
        let mut local_ticks = FILETIME::default();
        if SystemTimeToFileTime(&local, &mut local_ticks).is_err() {
            return 0;
        }
        let local_ticks =
            ((local_ticks.dwHighDateTime as i64) << 32) | local_ticks.dwLowDateTime as i64;
        (local_ticks - ticks) / 10_000_000
    }
}

/// The seal over a counter's values, for this machine.
fn seal(counter: &Counter, machine: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SEAL_SALT.as_bytes());
    hasher.update(machine.as_bytes());
    hasher.update(
        format!(
            "|{}|{}|{}|{}",
            counter.day, counter.used, counter.high_water_day, counter.first_seen
        )
        .as_bytes(),
    );
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
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
                seal: String::new(),
            },
            recent: Vec::new(),
            machine: "test-machine".into(),
        }
    }

    fn today_quota(used: u32) -> Quota {
        quota(used, local_day(), local_day())
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

    /// The allowance runs out, and says so with the numbers the bubble shows.
    #[test]
    fn the_tenth_translation_is_the_last_free_one_today() {
        let mut q = today_quota(0);
        for n in 0..FREE_DAILY_TRANSLATIONS {
            assert_eq!(q.verdict(&format!("text {n}"), &free()), Verdict::Allow);
            q.record(&format!("text {n}"));
        }
        assert!(matches!(
            q.verdict("something new", &free()),
            Verdict::Capped {
                used: 10,
                limit: 10
            },
        ));
    }

    /// A new day restores the allowance — reloaded the way a launch tomorrow
    /// would be, from what was written.
    #[test]
    fn tomorrow_starts_over() {
        let today = local_day();
        let mut q = quota(FREE_DAILY_TRANSLATIONS, today - 1, today - 1);
        let json = serde_json::to_string(&q.counter).unwrap();
        q.counter = serde_json::from_str(&json).unwrap();

        assert_eq!(q.verdict("anything", &free()), Verdict::Allow);
        assert_eq!(q.used_today(), 0);
        assert_eq!(q.remaining(&free()), Some(FREE_DAILY_TRANSLATIONS));
    }

    /// Winding the clock back does not hand out a second allowance — and the
    /// count survives the clock catching up again.
    #[test]
    fn a_clock_moved_backwards_does_not_refill_the_allowance() {
        let today = local_day();
        let mut q = today_quota(FREE_DAILY_TRANSLATIONS);

        // Yesterday, according to a clock that has been tampered with.
        q.counter.day = today - 3;
        q.roll();
        assert_eq!(
            q.used_today(),
            FREE_DAILY_TRANSLATIONS,
            "winding back refilled it"
        );

        // And back to the real date, which is still not a new day.
        q.counter.day = today - 3;
        q.counter.high_water_day = today;
        q.roll();
        assert_eq!(q.used_today(), FREE_DAILY_TRANSLATIONS);
        assert!(matches!(q.verdict("one", &free()), Verdict::Capped { .. }));
    }

    /// A counter written by the build that metered once per install still
    /// loads, and the flag that once meant unlimited use is simply ignored.
    #[test]
    fn an_old_counter_still_parses_and_its_legacy_flag_means_nothing() {
        let legacy = r#"{"used":99,"legacy_unlimited":true}"#;
        let q = Quota::open(Some(legacy), true, "m".into());
        assert_eq!(q.limit(&free()), Some(FREE_DAILY_TRANSLATIONS));
    }

    #[test]
    fn a_sealed_counter_is_trusted_on_the_machine_that_sealed_it() {
        let mut counter = Counter::default();
        counter.used = 3;
        counter.seal = seal(&counter, "m");
        let raw = serde_json::to_string(&counter).unwrap();
        let mut q = Quota::open(Some(&raw), true, "m".into());
        assert_eq!(q.used_today(), 3);
    }

    #[test]
    fn an_edited_or_foreign_counter_reads_as_a_spent_day() {
        let mut counter = Counter::default();
        counter.used = 3;
        counter.seal = seal(&counter, "m");
        let raw = serde_json::to_string(&counter).unwrap();
        // An old, unsealed counter is taken as it is.
        let unsealed = r#"{"day":0,"used":3}"#;
        let mut q = Quota::open(Some(unsealed), true, "m".into());
        assert_eq!(q.remaining(&free()), Some(FREE_DAILY_TRANSLATIONS));
        // Another machine's counter.
        let mut q = Quota::open(Some(&raw), true, "other".into());
        assert_eq!(q.remaining(&free()), Some(0));
        // The same counter with its count lowered by hand.
        let edited = raw.replace("\"used\":3", "\"used\":0");
        let mut q = Quota::open(Some(&edited), true, "m".into());
        assert_eq!(q.remaining(&free()), Some(0));
    }

    #[test]
    fn a_missing_counter_is_new_only_on_a_new_machine() {
        let mut fresh = Quota::open(None, false, "m".into());
        assert_eq!(fresh.remaining(&free()), Some(FREE_DAILY_TRANSLATIONS));
        let mut deleted = Quota::open(None, true, "m".into());
        assert_eq!(deleted.remaining(&free()), Some(0));
    }

    #[test]
    fn a_spent_day_is_only_today() {
        let today = local_day();
        let mut q = quota(0, today - 1, today - 1);
        q.spend_today();
        assert_eq!(q.remaining(&free()), Some(0));
        // Tomorrow starts over as usual.
        q.counter.day = today - 1;
        q.counter.high_water_day = today - 1;
        q.roll();
        assert_eq!(q.remaining(&free()), Some(FREE_DAILY_TRANSLATIONS));
    }

    #[test]
    fn pro_is_unlimited_whatever_the_counter_says() {
        let q = Quota::open(None, true, "m".into());
        assert_eq!(q.limit(&pro()), None);
    }

    /// Re-reading a sentence is one translation, not two. Without this, the
    /// language picker and an idle re-selection both cost the user quota.
    #[test]
    fn re_selecting_the_same_text_is_free() {
        let mut q = today_quota(0);
        q.record("merhaba dünya");
        // Spend the rest of the day on other things.
        for n in 0..(FREE_DAILY_TRANSLATIONS - 1) {
            q.record(&format!("other {n}"));
        }
        assert_eq!(q.used_today(), FREE_DAILY_TRANSLATIONS);
        assert_eq!(q.verdict("merhaba dünya", &free()), Verdict::Repeat);
        // Whitespace differences are the same selection to a human.
        assert_eq!(q.verdict("  merhaba dünya  ", &free()), Verdict::Repeat);
        assert!(matches!(
            q.verdict("a different sentence", &free()),
            Verdict::Capped { .. },
        ));
    }

    /// Every refusal carries the numbers the bubble shows, every time. There
    /// is no throttle to fall silent behind.
    #[test]
    fn every_refusal_offers_the_way_out() {
        let mut q = today_quota(FREE_DAILY_TRANSLATIONS);
        for n in 0..5 {
            assert!(matches!(
                q.verdict(&format!("try {n}"), &free()),
                Verdict::Capped {
                    used: 10,
                    limit: 10
                },
            ));
        }
    }

    /// A count that somehow exceeds the limit shows zero left, not a wrap.
    #[test]
    fn remaining_never_underflows() {
        let mut q = today_quota(FREE_DAILY_TRANSLATIONS + 5);
        assert_eq!(q.remaining(&free()), Some(0));
        assert_eq!(q.remaining(&pro()), None);
    }

    /// Pro is unlimited however much has been counted.
    #[test]
    fn unlimited_means_unlimited() {
        let mut q = today_quota(9_000);
        assert_eq!(q.verdict("anything", &pro()), Verdict::Allow);
    }
}
