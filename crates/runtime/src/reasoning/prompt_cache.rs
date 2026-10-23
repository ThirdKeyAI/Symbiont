//! Optional provider prompt caching and advisory cache usage.
//!
//! Raw token accounting remains authoritative. Missing cache counts are unknown,
//! not confirmed zero, and must not be used to release a cost reservation.

use super::inference::{InferenceError, InferenceOptions, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const OPTIONS_KEY: &str = "symbi_prompt_cache";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCache {
    /// Do not send runtime cache markers. Provider-implicit caching may remain.
    Off,
    /// Mark the static prefix where the provider supports it.
    Prefix,
    /// Mark the static prefix and enable automatic conversation caching.
    Rolling,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtl {
    #[default]
    FiveMinutes,
    OneHour,
}

#[cfg(feature = "cloud-llm")]
impl CacheTtl {
    pub(crate) fn marker(self) -> Value {
        match self {
            Self::FiveMinutes => serde_json::json!({"type":"ephemeral"}),
            Self::OneHour => serde_json::json!({"type":"ephemeral","ttl":"1h"}),
        }
    }
}

/// Explicit configuration. Absence preserves the historical provider behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCacheConfig {
    pub mode: PromptCache,
    #[serde(default)]
    pub ttl: CacheTtl,
}

impl InferenceOptions {
    /// Set typed cache options without adding fields to the public options struct.
    /// The reserved entry is hashed with the inference contract, never forwarded.
    pub fn set_prompt_cache(&mut self, config: PromptCacheConfig) {
        self.extra.insert(
            OPTIONS_KEY.into(),
            serde_json::to_value(config).expect("cache configuration is serializable"),
        );
    }

    pub fn with_prompt_cache(mut self, config: PromptCacheConfig) -> Self {
        self.set_prompt_cache(config);
        self
    }

    pub fn prompt_cache(&self) -> Result<Option<PromptCacheConfig>, InferenceError> {
        self.extra
            .get(OPTIONS_KEY)
            .map(|value| {
                serde_json::from_value(value.clone()).map_err(|_| {
                    InferenceError::InvalidRequest("invalid prompt cache configuration".into())
                })
            })
            .transpose()
    }
}

/// Reported subsets of raw prompt tokens. None means absent or null upstream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheUsage {
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
}

/// Complete raw accounting plus optional cache details, for custom transports.
/// This additive type leaves the existing Usage and response wire formats intact.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub usage: Usage,
    pub cache: CacheUsage,
}

impl ProviderUsage {
    pub fn from_anthropic(value: Option<&Value>) -> Result<Self, InferenceError> {
        Self::parse(value, true)
    }

    pub fn from_chat_completions(value: Option<&Value>) -> Result<Self, InferenceError> {
        Self::parse(value, false)
    }

    fn parse(value: Option<&Value>, anthropic: bool) -> Result<Self, InferenceError> {
        let Some(value) = value.filter(|v| !v.is_null()) else {
            return Ok(Self::default());
        };
        if !value.is_object() {
            return Err(InferenceError::ParseError("usage must be an object".into()));
        }
        let count = |parent: &Value, key: &str, nullable: bool| {
            parent
                .get(key)
                .filter(|v| !nullable || !v.is_null())
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| {
                            InferenceError::ParseError(format!("invalid token count: {key}"))
                        })
                })
                .transpose()
        };
        let (prompt, completion, total, cache) = if anthropic {
            let cache = CacheUsage {
                cache_read_tokens: count(value, "cache_read_input_tokens", true)?,
                cache_creation_tokens: count(value, "cache_creation_input_tokens", true)?,
            };
            let (Some(input), Some(output)) = (
                count(value, "input_tokens", false)?,
                count(value, "output_tokens", false)?,
            ) else {
                return Ok(Self::default());
            };
            let input = input
                .checked_add(cache.cache_read_tokens.unwrap_or(0))
                .and_then(|n| n.checked_add(cache.cache_creation_tokens.unwrap_or(0)))
                .ok_or_else(|| InferenceError::ParseError("input token count overflow".into()))?;
            let total = input
                .checked_add(output)
                .ok_or_else(|| InferenceError::ParseError("total token count overflow".into()))?;
            (input, output, total, cache)
        } else {
            let details = value.get("prompt_tokens_details").filter(|v| !v.is_null());
            if details.is_some_and(|v| !v.is_object()) {
                return Err(InferenceError::ParseError(
                    "prompt token details must be an object".into(),
                ));
            }
            let cache = match details {
                Some(d) => CacheUsage {
                    cache_read_tokens: count(d, "cached_tokens", true)?,
                    cache_creation_tokens: count(d, "cache_write_tokens", true)?,
                },
                None => CacheUsage::default(),
            };
            let (Some(input), Some(output), Some(total)) = (
                count(value, "prompt_tokens", false)?,
                count(value, "completion_tokens", false)?,
                count(value, "total_tokens", false)?,
            ) else {
                return Ok(Self::default());
            };
            if input.checked_add(output).is_none_or(|sum| sum > total) {
                return Err(InferenceError::ParseError(
                    "inconsistent total token count".into(),
                ));
            }
            let cached = cache
                .cache_read_tokens
                .unwrap_or(0)
                .checked_add(cache.cache_creation_tokens.unwrap_or(0));
            if cached.is_none_or(|n| n > input) {
                return Err(InferenceError::ParseError(
                    "cache counts exceed prompt tokens".into(),
                ));
            }
            (input, output, total, cache)
        };
        Ok(Self {
            usage: Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: total,
            },
            cache,
        })
    }
}

/// One successful provider response, without prompts, credentials or tool data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheUsageObservation {
    pub provider: String,
    pub model: String,
    pub reported: ProviderUsage,
}

/// Advisory synchronous telemetry. Implementations should return promptly and
/// never panic. This callback is not a durable audit or budget-settlement hook.
pub type CacheUsageObserver = dyn Fn(&CacheUsageObservation) + Send + Sync;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_usage_preserves_raw_totals_on_both_protocols() {
        let direct = ProviderUsage::from_anthropic(Some(&json!({
            "input_tokens": 10, "output_tokens": 5,
            "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000
        })))
        .unwrap();
        assert_eq!(
            direct.usage,
            Usage {
                prompt_tokens: 1110,
                completion_tokens: 5,
                total_tokens: 1115
            }
        );
        assert_eq!(direct.cache.cache_read_tokens, Some(1000));
        assert_eq!(direct.cache.cache_creation_tokens, Some(100));
        let routed = ProviderUsage::from_chat_completions(Some(&json!({
            "prompt_tokens":10339,"completion_tokens":60,"total_tokens":10399,
            "prompt_tokens_details":{"cached_tokens":10318,"cache_write_tokens":0}
        })))
        .unwrap();
        assert_eq!(routed.usage.prompt_tokens, 10339);
        assert_eq!(routed.cache.cache_read_tokens, Some(10318));
        assert_eq!(routed.cache.cache_creation_tokens, Some(0));
    }

    #[test]
    fn missing_and_null_cache_counts_are_unknown_not_zero() {
        for details in [
            Value::Null,
            json!({}),
            json!({"cached_tokens":null,"cache_write_tokens":null}),
        ] {
            let report = ProviderUsage::from_chat_completions(Some(&json!({
                "prompt_tokens":10,"completion_tokens":1,"total_tokens":11,
                "prompt_tokens_details":details
            })))
            .unwrap();
            assert_eq!(report.cache, CacheUsage::default());
            assert_eq!(report.usage.total_tokens, 11);
        }
        let partial = ProviderUsage::from_chat_completions(Some(&json!({
            "prompt_tokens":10,"completion_tokens":1,"total_tokens":11,
            "prompt_tokens_details":{"cached_tokens":5}
        })))
        .unwrap();
        assert_eq!(partial.cache.cache_creation_tokens, None);
        assert_eq!(partial.cache.cache_read_tokens, Some(5));
        assert_eq!(
            ProviderUsage::from_anthropic(None).unwrap(),
            ProviderUsage::default()
        );
    }

    #[test]
    fn malformed_or_inflated_cache_counts_are_rejected() {
        for details in [
            json!(12),
            json!([]),
            json!({"cached_tokens":-1}),
            json!({"cache_write_tokens":1.5}),
            json!({"cached_tokens":"1"}),
            json!({"cached_tokens":4294967296u64}),
            json!({"cached_tokens":9,"cache_write_tokens":2}),
            json!({"cached_tokens":4294967295u64,"cache_write_tokens":4294967295u64}),
        ] {
            assert!(ProviderUsage::from_chat_completions(Some(&json!({
                "prompt_tokens":10,"completion_tokens":1,"total_tokens":11,
                "prompt_tokens_details":details
            })))
            .is_err());
        }
        assert!(ProviderUsage::from_anthropic(Some(&json!({
            "input_tokens":4294967295u64,"output_tokens":0,"cache_read_input_tokens":1
        })))
        .is_err());
    }

    #[test]
    fn legacy_usage_and_default_options_keep_their_serialized_shape() {
        assert_eq!(
            serde_json::to_value(Usage::default()).unwrap(),
            json!({
                "prompt_tokens":0,"completion_tokens":0,"total_tokens":0
            })
        );
        let options = InferenceOptions::default();
        assert!(serde_json::to_value(&options)
            .unwrap()
            .get("extra")
            .is_none());
        assert_eq!(options.prompt_cache().unwrap(), None);
        let configured = options.with_prompt_cache(PromptCacheConfig {
            mode: PromptCache::Rolling,
            ttl: CacheTtl::FiveMinutes,
        });
        let serialized = serde_json::to_value(&configured).unwrap();
        assert_eq!(serialized["extra"][OPTIONS_KEY]["mode"], "rolling");
        assert_eq!(serialized["extra"][OPTIONS_KEY]["ttl"], "five_minutes");
        let decoded: InferenceOptions = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            decoded.prompt_cache().unwrap(),
            configured.prompt_cache().unwrap()
        );
    }

    #[test]
    fn invalid_typed_cache_policy_cannot_silently_fall_back() {
        for config in [
            Value::Null,
            json!({"mode":"typo"}),
            json!({"mode":"rolling","ttl":"forever"}),
            json!({"mode":"off","unknown":true}),
        ] {
            let mut options = InferenceOptions::default();
            options.extra.insert(OPTIONS_KEY.into(), config);
            assert!(options.prompt_cache().is_err());
        }
    }
}
