//! Daily counts waiting for the next ping.
//!
//! Pings flush these counts every five minutes. Counts remain under the local
//! day they happened, including across restarts and offline days. Only after
//! a successful service response are the sent amounts subtracted; counts added
//! while a request was in flight stay for the next delivery.
//!
//! A count is a section, a name and a field: `claude` / `claude-haiku-5-5` /
//! `input`, `providers` / `Google` / `ok`, `screen` / `page` / `done`. Counts
//! only — nothing that was selected, read or translated ever comes here.
//!
//! Deliberately outside the sealed counter in [`crate::quota`]: nothing here
//! decides what anyone may do, and a tidied-up stats file must never cost
//! someone their allowance.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::quota::local_day;

/// Several threads count at once — the screen asks Claude about boxes in
/// parallel — and every one of them reads, adds to and writes the same file.
static FILE: Mutex<()> = Mutex::new(());

/// How many days of undelivered counts are kept. A copy that is never allowed
/// to ping keeps a month and drops the rest rather than growing without end.
const KEEP_DAYS: i64 = 31;

/// field -> count
pub type Fields = BTreeMap<String, u64>;

/// One delivery unit: a day's counts for one name in one section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub day: i64,
    pub section: String,
    pub name: String,
    pub fields: Fields,
}

/// day -> section -> name -> fields. The day is a string only because JSON
/// object keys are.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Log {
    #[serde(default)]
    days: BTreeMap<String, BTreeMap<String, BTreeMap<String, Fields>>>,
}

/// Adds to today's counts.
pub fn add(section: &str, name: &str, fields: &[(&str, u64)]) {
    let _guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = load();
    put(&mut log, local_day(), section, name, fields);
    save(&log);
}

/// Everything not yet delivered.
pub fn unsent() -> Vec<Entry> {
    let _guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    flatten(&load())
}

/// Takes what a ping just delivered off the books, and only that.
pub fn delivered(sent: &[Entry]) {
    let _guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = load();
    settle(&mut log, sent);
    save(&log);
}

/// The entries as the ping sends them.
pub fn to_json(entries: &[Entry]) -> serde_json::Value {
    entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "day": e.day,
                "section": e.section,
                "name": e.name,
                "counts": e.fields,
            })
        })
        .collect()
}

fn put(log: &mut Log, day: i64, section: &str, name: &str, fields: &[(&str, u64)]) {
    let slot = log
        .days
        .entry(day.to_string())
        .or_default()
        .entry(section.to_string())
        .or_default()
        .entry(name.to_string())
        .or_default();
    for (field, n) in fields {
        if *n > 0 {
            *slot.entry((*field).to_string()).or_default() += n;
        }
    }
}

fn flatten(log: &Log) -> Vec<Entry> {
    let mut out = Vec::new();
    for (day, sections) in &log.days {
        let Ok(day) = day.parse::<i64>() else {
            continue;
        };
        for (section, names) in sections {
            for (name, fields) in names {
                let fields: Fields = fields
                    .iter()
                    .filter(|(_, n)| **n > 0)
                    .map(|(k, n)| (k.clone(), *n))
                    .collect();
                if !fields.is_empty() {
                    out.push(Entry {
                        day,
                        section: section.clone(),
                        name: name.clone(),
                        fields,
                    });
                }
            }
        }
    }
    out
}

/// Subtracts what was sent, saturating — the file may have been edited or
/// reset in between — and forgets whatever is left at zero.
fn settle(log: &mut Log, sent: &[Entry]) {
    for e in sent {
        let day = e.day.to_string();
        let Some(sections) = log.days.get_mut(&day) else {
            continue;
        };
        if let Some(names) = sections.get_mut(&e.section) {
            if let Some(fields) = names.get_mut(&e.name) {
                for (field, n) in &e.fields {
                    if let Some(left) = fields.get_mut(field) {
                        *left = left.saturating_sub(*n);
                    }
                }
                fields.retain(|_, n| *n > 0);
                if fields.is_empty() {
                    names.remove(&e.name);
                }
            }
            if names.is_empty() {
                sections.remove(&e.section);
            }
        }
        if sections.is_empty() {
            log.days.remove(&day);
        }
    }
}

fn load() -> Log {
    let mut log: Log = std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let oldest = local_day() - KEEP_DAYS;
    log.days
        .retain(|day, _| day.parse::<i64>().is_ok_and(|d| d > oldest));
    log
}

/// Best-effort: a counter that cannot be written must never stop a
/// translation, so every failure here is discarded on purpose.
fn save(log: &Log) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(raw) = serde_json::to_string_pretty(log) {
        let _ = std::fs::write(path, raw);
    }
}

fn path() -> PathBuf {
    if let Some(home) = crate::config::state_home() {
        return home.join("outbox.json");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bubbleTranslate")
        .join("outbox.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivering_takes_off_only_what_was_sent() {
        let mut log = Log::default();
        let day = local_day();
        put(
            &mut log,
            day,
            "claude",
            "claude-haiku-5-5",
            &[("requests", 3), ("input", 3000)],
        );
        put(&mut log, day - 1, "providers", "Google", &[("ok", 40)]);
        let sent = flatten(&log);
        assert_eq!(sent.len(), 2);
        // One more answered while the ping was on its way.
        put(
            &mut log,
            day,
            "claude",
            "claude-haiku-5-5",
            &[("requests", 1), ("input", 1000)],
        );

        settle(&mut log, &sent);
        let left = flatten(&log);
        assert_eq!(
            left,
            vec![Entry {
                day,
                section: "claude".into(),
                name: "claude-haiku-5-5".into(),
                fields: [("input".to_string(), 1000), ("requests".to_string(), 1)].into(),
            }]
        );

        settle(&mut log, &left);
        assert!(log.days.is_empty(), "a delivered day is forgotten");
    }

    #[test]
    fn zero_counts_are_not_kept() {
        let mut log = Log::default();
        put(&mut log, local_day(), "screen", "page", &[("done", 0)]);
        assert!(flatten(&log).is_empty());
    }
}
