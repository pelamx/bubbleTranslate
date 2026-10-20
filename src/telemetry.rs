//! Content-free daily cumulative counters. Replayed snapshots are idempotent.
use serde::{Deserialize, Serialize};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
pub struct Counter {
    day: i64,
    app: String,
    #[serde(default = "channel")]
    channel: String,
    feature: String,
    outcome: String,
    count: u64,
}

fn channel() -> String {
    if cfg!(feature = "store") {
        "store"
    } else {
        "download"
    }
    .into()
}

pub fn enable(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

fn path() -> std::path::PathBuf {
    crate::config::state_home()
        .unwrap_or_else(|| {
            crate::config::Config::path()
                .parent()
                .unwrap()
                .to_path_buf()
        })
        .join("metrics.json")
}

fn load() -> Vec<Counter> {
    let mut rows: Vec<Counter> = std::fs::read(path())
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    let today = crate::quota::local_day();
    rows.retain(|r| r.day > today - 31 && r.day <= today + 1);
    rows
}

pub fn record(feature: &str, outcome: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // Also used by the panic hook: never wait on a lock a panicking thread owns.
    let guard = if std::thread::panicking() {
        FILE.try_lock().ok()
    } else {
        FILE.lock().ok()
    };
    let Some(_guard) = guard else {
        return;
    };
    let mut rows = load();
    let day = crate::quota::local_day();
    increment(&mut rows, day, feature, outcome);
    if let Ok(raw) = serde_json::to_vec(&rows) {
        let path = path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, raw).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

fn increment(rows: &mut Vec<Counter>, day: i64, feature: &str, outcome: &str) {
    if let Some(row) = rows.iter_mut().find(|r| {
        r.day == day
            && r.app == env!("CARGO_PKG_VERSION")
            && r.channel == channel()
            && r.feature == feature
            && r.outcome == outcome
    }) {
        row.count = row.count.saturating_add(1);
    } else {
        rows.push(Counter {
            day,
            app: env!("CARGO_PKG_VERSION").into(),
            channel: channel(),
            feature: feature.into(),
            outcome: outcome.into(),
            count: 1,
        });
    }
}

pub fn snapshot() -> Vec<Counter> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let Ok(_guard) = FILE.lock() else {
        return Vec::new();
    };
    load()
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record("app", "panic");
        previous(info);
    }));
}

/// One failed request, plus its distinct fixed reason codes. No error strings.
pub fn failed(
    feature: &str,
    errors: &[(crate::config::Provider, crate::translate::TranslateError)],
) {
    use crate::translate::TranslateError;
    record(feature, "translation_failed");
    let codes: std::collections::BTreeSet<_> = errors
        .iter()
        .map(|(_, error)| match error {
            TranslateError::RateLimited | TranslateError::Http(429) => "rate_limited",
            TranslateError::Network(_) => "network_failed",
            TranslateError::BadResponse(_) => "bad_response",
            TranslateError::Unavailable(_) => "provider_unavailable",
            TranslateError::Http(_) => "http_failed",
        })
        .collect();
    for code in codes {
        record(feature, code);
    }
}

pub fn source(via: crate::platform::CaptureSource) -> &'static str {
    use crate::platform::CaptureSource::*;
    match via {
        Accessibility => "selection",
        PrimarySelection => "selection",
        Clipboard => "clipboard",
        Ocr => "ocr_box",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_counts_survive_serialization_and_keep_outcomes_and_days_separate() {
        let mut rows = Vec::new();
        increment(&mut rows, 10, "selection", "ok");
        increment(&mut rows, 10, "selection", "ok");
        increment(&mut rows, 10, "selection", "translation_failed");
        increment(&mut rows, 11, "ocr_box", "ok");
        let raw = serde_json::to_vec(&rows).unwrap();
        let mut loaded: Vec<Counter> = serde_json::from_slice(&raw).unwrap();
        increment(&mut loaded, 10, "selection", "ok");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].count, 3);
        assert_eq!(loaded[1].count, 1);
        assert_eq!(loaded[2].count, 1);
        assert!(!String::from_utf8(raw).unwrap().contains("text"));
    }
    #[test]
    fn counts_never_overflow() {
        let mut rows = Vec::new();
        increment(&mut rows, 10, "selection", "ok");
        rows[0].count = u64::MAX;
        increment(&mut rows, 10, "selection", "ok");
        assert_eq!(rows[0].count, u64::MAX);
    }
}
