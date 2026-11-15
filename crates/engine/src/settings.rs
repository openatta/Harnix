//! What the engine's settings files say, checked in the engine's own terms.

use serde_json::Value;

/// Whether `entry` is a provider the engine can read from its settings: `Err` says why not, in
/// the engine's words.
pub fn check_provider(entry: &Value) -> Result<(), String> {
    serde_json::from_value::<base::provider::ProviderConfig>(entry.clone())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_provider_the_engine_cannot_read_says_why() {
        assert!(check_provider(&json!({"api_type": "anthropic"})).is_ok());
        assert!(check_provider(&json!({"api_type": "anthropic", "base_url": 3})).is_err());
    }
}
