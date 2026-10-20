//! Bounded remote overrides; local settings are never saved with overrides.
use crate::config::{Config, Provider};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    debounce_ms: Option<u64>,
    translation_timeout_ms: Option<u64>,
    disabled_providers: Vec<Provider>,
    dictionary_enabled: Option<bool>,
    update_message: Option<String>,
    update_before_version: Option<String>,
}
static CURRENT: Mutex<Option<(Instant, Settings)>> = Mutex::new(None);

fn settings() -> Settings {
    CURRENT
        .lock()
        .ok()
        .map(|s| fresh(s.as_ref()))
        .unwrap_or_default()
}

fn fresh(saved: Option<&(Instant, Settings)>) -> Settings {
    saved
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(86400))
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
}

pub fn apply(cfg: &mut Config) {
    apply_settings(cfg, &settings());
}

fn apply_settings(cfg: &mut Config, s: &Settings) {
    if let Some(ms) = s.debounce_ms.filter(|ms| (0..=2000).contains(ms)) {
        cfg.debounce_ms = ms;
    }
    cfg.providers.retain(|p| !s.disabled_providers.contains(p));
    if s.disabled_providers.contains(&Provider::Google) {
        cfg.dictionary = false;
    }
    if s.disabled_providers.contains(&Provider::Claude) {
        cfg.claude_reads_screen = false;
        cfg.claude_word_context = false;
    }
    if s.dictionary_enabled == Some(false) {
        cfg.dictionary = false;
    }
}
pub fn translation_timeout() -> Duration {
    Duration::from_millis(
        settings()
            .translation_timeout_ms
            .filter(|v| (2000..=30000).contains(v))
            .unwrap_or(10000),
    )
}

fn version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.split('.');
    let value = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(value)
}

pub fn update_message() -> Option<String> {
    message_for(settings(), env!("CARGO_PKG_VERSION"))
}

fn message_for(s: Settings, running: &str) -> Option<String> {
    if let Some(before) = s.update_before_version.as_deref()
        && version(running)? >= version(before)?
    {
        return None;
    }
    s.update_message.filter(|s| !s.is_empty())
}

pub fn refresh(agent: &ureq::Agent) {
    #[cfg(debug_assertions)]
    if std::env::var_os("BUBBLETRANSLATE_LICENSE_API").is_none() {
        return;
    }
    let url = format!(
        "{}/v1/config?os={}&channel={}",
        crate::license::api_base(),
        std::env::consts::OS,
        if cfg!(feature = "store") {
            "store"
        } else {
            "download"
        }
    );
    let Ok(mut response) = agent.get(url).call() else {
        return;
    };
    if !response.status().is_success() {
        return;
    }
    let Ok(raw) = response
        .body_mut()
        .with_config()
        .limit(4096)
        .read_to_string()
    else {
        return;
    };
    let Some(s) = parse_settings(&raw) else {
        return;
    };
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some((Instant::now(), s));
    }
}

fn parse_settings(raw: &str) -> Option<Settings> {
    let s: Settings = serde_json::from_str(raw).ok()?;
    if s.translation_timeout_ms
        .is_some_and(|v| !(2000..=30000).contains(&v))
        || s.disabled_providers.len() > 4
        || s.debounce_ms.is_some_and(|v| v > 2000)
        || s.update_before_version
            .as_deref()
            .is_some_and(|v| version(v).is_none())
        || s.update_message
            .as_ref()
            .is_some_and(|v| v.chars().count() > 300)
    {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_only_the_request_copy_and_disables_claude_special_paths() {
        let local = Config::default();
        let mut request = local.clone();
        let overrides = parse_settings(r#"{"debounce_ms":400,"disabled_providers":["google","claude"],"dictionary_enabled":false}"#).unwrap();
        apply_settings(&mut request, &overrides);
        assert_eq!(request.debounce_ms, 400);
        assert!(!request.dictionary);
        assert!(!request.providers.contains(&Provider::Google));
        assert!(!request.screen_goes_to_claude());
        assert!(!request.word_goes_to_claude());
        assert!(local.providers.contains(&Provider::Google));
        assert_eq!(local.debounce_ms, Config::default().debounce_ms);
    }
    #[test]
    fn rejects_out_of_range_values_and_unknown_providers() {
        for raw in [
            r#"{"debounce_ms":2001}"#,
            r#"{"translation_timeout_ms":1999}"#,
            r#"{"translation_timeout_ms":30001}"#,
            r#"{"disabled_providers":["unknown"]}"#,
            r#"{"update_before_version":"bad"}"#,
        ] {
            assert!(parse_settings(raw).is_none(), "accepted {raw}");
        }
        assert!(
            parse_settings(r#"{"translation_timeout_ms":2000,"update_before_version":"0.4.22"}"#)
                .is_some()
        );
        assert!(version("0.4.9") < version("0.4.20"));
    }
    #[test]
    fn update_message_only_targets_older_versions() {
        let settings = parse_settings(
            r#"{"update_message":"Please update","update_before_version":"0.4.20"}"#,
        )
        .unwrap();
        assert_eq!(
            message_for(settings.clone(), "0.4.9"),
            Some("Please update".into())
        );
        assert_eq!(message_for(settings.clone(), "0.4.20"), None);
        assert_eq!(message_for(settings, "0.4.21"), None);
    }
    #[test]
    fn expired_overrides_revert_to_defaults() {
        let saved = Settings {
            debounce_ms: Some(500),
            ..Default::default()
        };
        assert_eq!(
            fresh(Some(&(Instant::now(), saved.clone()))).debounce_ms,
            Some(500)
        );
        assert_eq!(
            fresh(Some(&(Instant::now() - Duration::from_secs(86401), saved))).debounce_ms,
            None
        );
        assert_eq!(fresh(None).debounce_ms, None);
    }
    #[test]
    fn rejects_unknown_controls_and_wrong_types() {
        assert!(serde_json::from_str::<Settings>(r#"{"shell":"command"}"#).is_err());
        assert!(serde_json::from_str::<Settings>(r#"{"dictionary_enabled":"false"}"#).is_err());
        assert!(serde_json::from_str::<Settings>(r#"{"debounce_ms":-1}"#).is_err());
    }
}
