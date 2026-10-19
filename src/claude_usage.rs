//! How much this copy asked of Claude.
//!
//! Claude is the one backend paid by the token, on the user's own key, and
//! the screen is read through it by default once a key is set. What that
//! costs people is a question nobody here could otherwise answer: the
//! requests go from this machine straight to Anthropic and never pass the
//! service. So each answer's token counts go into the [`crate::outbox`] under
//! the model's name, and the daily ping carries them. Counts only — never the
//! text, the picture or the key.

/// One answered request: the `usage` object of a Messages API reply.
pub fn answered(model: &str, usage: Option<&serde_json::Value>) {
    crate::outbox::add("claude", model, &counts(usage));
}

/// One request that got no answer: a refused key, no credit, no network.
pub fn failed(model: &str) {
    crate::outbox::add("claude", model, &[("failed", 1)]);
}

fn counts(usage: Option<&serde_json::Value>) -> [(&'static str, u64); 5] {
    let count = |field: &str| {
        usage
            .and_then(|u| u.get(field))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    [
        ("requests", 1),
        ("input", count("input_tokens")),
        ("output", count("output_tokens")),
        ("cache_read", count("cache_read_input_tokens")),
        ("cache_write", count("cache_creation_input_tokens")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_usage_of_a_reply() {
        let usage = serde_json::json!({
            "input_tokens": 1834,
            "output_tokens": 212,
            "cache_read_input_tokens": 0,
        });
        assert_eq!(
            counts(Some(&usage)),
            [
                ("requests", 1),
                ("input", 1834),
                ("output", 212),
                ("cache_read", 0),
                ("cache_write", 0)
            ]
        );
        // A reply without usage still counts as a request.
        assert_eq!(counts(None)[0], ("requests", 1));
    }
}
