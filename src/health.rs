//! Which backends answered today, and which did not.
//!
//! This exists because the default translation path is the weakest thing in the
//! app and the one nobody would hear about. The Google endpoint it leads with is
//! undocumented and unauthenticated — see the note above `Translator::google` —
//! so the day it starts refusing, every free install quietly falls through to a
//! worse translation and the first anyone here learns of it is a support mail.
//!
//! Deliberately *not* part of the sealed counter in [`crate::quota`]. That seal
//! is there so an edited allowance is caught; folding diagnostics into it would
//! mean someone tidying up a stats file lost their free translations. Nothing
//! here is worth protecting: a tampered tally misleads no one but whoever reads
//! the totals.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Provider;
use crate::quota::local_day;

/// Today's record, keyed by the provider's own label so the file stays readable
/// and a provider that is renamed or removed does not make it unparseable.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Log {
    /// Days since the epoch, in local time. A different day resets the tallies.
    #[serde(default)]
    day: i64,
    #[serde(default)]
    providers: BTreeMap<String, Tally>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Tally {
    /// Translations this provider produced.
    #[serde(default)]
    pub ok: u32,
    /// Times it was asked and could not answer.
    #[serde(default)]
    pub failed: u32,
}

pub struct Health {
    log: Log,
}

impl Health {
    pub fn load() -> Self {
        let mut log: Log = std::fs::read_to_string(path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let today = local_day();
        if log.day != today {
            log = Log {
                day: today,
                providers: BTreeMap::new(),
            };
        }
        Self { log }
    }

    /// Records that a provider answered.
    ///
    /// Only the one that actually produced the translation is counted, not the
    /// ones tried before it — the chain does not hand those back. That is
    /// enough for the question this is here to answer: the fallbacks normally
    /// win nothing at all, so a day where MyMemory's count climbs is a day
    /// Google stopped answering, without needing to be told so directly.
    pub fn answered(&mut self, provider: Provider) {
        self.entry(provider).ok += 1;
        self.save();
    }

    /// Records that every provider in the chain refused.
    pub fn refused(&mut self, providers: impl IntoIterator<Item = Provider>) {
        for provider in providers {
            self.entry(provider).failed += 1;
        }
        self.save();
    }

    /// Today's tallies, in the providers' own order.
    pub fn today(&self) -> Vec<(Provider, Tally)> {
        Provider::ALL
            .iter()
            .filter_map(|p| {
                self.log
                    .providers
                    .get(p.label())
                    .filter(|t| t.ok > 0 || t.failed > 0)
                    .map(|t| (*p, *t))
            })
            .collect()
    }

    fn entry(&mut self, provider: Provider) -> &mut Tally {
        self.log
            .providers
            .entry(provider.label().to_string())
            .or_default()
    }

    /// Best-effort: a diagnostics file that cannot be written must never stop a
    /// translation, so every failure here is discarded on purpose.
    fn save(&self) {
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string_pretty(&self.log) {
            let _ = std::fs::write(path, raw);
        }
    }
}

fn path() -> PathBuf {
    if let Some(home) = crate::config::state_home() {
        return home.join("health.json");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bubbleTranslate")
        .join("health.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Health {
        Health {
            log: Log {
                day: local_day(),
                providers: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn a_provider_that_was_never_asked_is_not_listed() {
        let mut health = fresh();
        health.log.providers.clear();
        health.entry(Provider::Google).ok += 1;
        let today = health.today();
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].0, Provider::Google);
        assert_eq!(today[0].1.ok, 1);
    }

    /// The signal worth reading: the fallback winning anything at all means the
    /// provider ahead of it stopped answering.
    #[test]
    fn tallies_are_kept_per_provider_and_in_chain_order() {
        let mut health = fresh();
        health.entry(Provider::MyMemory).ok += 3;
        health.entry(Provider::Google).failed += 3;
        let today = health.today();
        assert_eq!(
            today.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            vec![Provider::Google, Provider::MyMemory],
            "reported in the order the chain tries them, not the order seen"
        );
        assert_eq!(today[0].1.failed, 3);
        assert_eq!(today[1].1.ok, 3);
    }

    /// Yesterday's numbers must not be read as today's, or a provider that
    /// broke last week looks broken forever.
    #[test]
    fn a_log_from_another_day_starts_empty() {
        let mut health = fresh();
        health.entry(Provider::Google).ok += 9;
        health.log.day = local_day() - 1;

        // What `load` does once it sees the day has moved.
        let stale = health.log.day != local_day();
        assert!(stale);
        let rolled = Log {
            day: local_day(),
            providers: BTreeMap::new(),
        };
        assert!(rolled.providers.is_empty());
    }
}
