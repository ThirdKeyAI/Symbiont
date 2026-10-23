use super::*;
use crate::reasoning::conversation::{ConversationMessage, ToolCall};
use crate::reasoning::prompt_cache::CacheTtl;
use serde_json::json;
use serial_test::serial;

fn provider(kind: &str) -> CloudInferenceProvider {
    let names = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_BASE_URL",
        "OPENROUTER_API_KEY",
        "OPENROUTER_MODEL",
        "OPENROUTER_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_MODEL",
        "OPENAI_BASE_URL",
        "BEDROCK_MODEL_ID",
    ];
    let previous: Vec<_> = names
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect();
    for key in names {
        std::env::remove_var(key);
    }
    std::env::set_var(format!("{kind}_API_KEY"), "synthetic-cache-fixture");
    std::env::set_var(
        format!("{kind}_MODEL"),
        if kind == "OPENROUTER" {
            "anthropic/fixture"
        } else {
            "fixture"
        },
    );
    let result = CloudInferenceProvider::from_env();
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    result.unwrap()
}

fn options(mode: PromptCache, ttl: CacheTtl) -> InferenceOptions {
    InferenceOptions::default().with_prompt_cache(PromptCacheConfig { mode, ttl })
}

#[test]
#[serial]
fn cache_controls_preserve_legacy_bodies_until_explicitly_selected() {
    let conversation = Conversation::with_system("Stable policy");
    for kind in ["ANTHROPIC", "OPENROUTER", "OPENAI"] {
        let client = provider(kind);
        let options = InferenceOptions::default();
        let legacy = if kind == "ANTHROPIC" {
            client.build_anthropic_body(&conversation, &options)
        } else {
            client.build_openai_body(&conversation, &options)
        };
        assert_eq!(
            client.build_request_body(&conversation, &options).unwrap(),
            legacy
        );
    }
}

#[test]
#[serial]
fn cache_modes_and_ttls_only_use_api_marker_positions() {
    for kind in ["ANTHROPIC", "OPENROUTER"] {
        let client = provider(kind);
        for with_system in [false, true] {
            for mode in [PromptCache::Off, PromptCache::Prefix, PromptCache::Rolling] {
                for ttl in [CacheTtl::FiveMinutes, CacheTtl::OneHour] {
                    let mut conv = if with_system {
                        Conversation::with_system("Stable policy")
                    } else {
                        Conversation::new()
                    };
                    conv.push(ConversationMessage::user("Review evidence"));
                    conv.push(ConversationMessage::assistant_tool_calls(vec![ToolCall {
                        id: "one".into(),
                        name: "lookup".into(),
                        arguments: r#"{"cache_control":"business data"}"#.into(),
                    }]));
                    conv.push(ConversationMessage::tool_result("one", "lookup", "Result"));
                    let mut opts = options(mode, ttl);
                    opts.tool_definitions.push(ToolDefinition {
                        name: "lookup".into(),
                        description: "Lookup".into(),
                        parameters: json!({"type":"object"}),
                    });
                    let body = client.build_request_body(&conv, &opts).unwrap();
                    assert!(body.get(OPTIONS_KEY).is_none());
                    let expected = (mode == PromptCache::Rolling).then(|| ttl.marker());
                    assert_eq!(body.get("cache_control"), expected.as_ref());
                    if kind == "ANTHROPIC" {
                        let prefix = if with_system {
                            &body["system"][0]
                        } else {
                            &body["tools"][0]
                        };
                        assert_eq!(
                            prefix.get("cache_control"),
                            if mode == PromptCache::Off {
                                None
                            } else {
                                Some(ttl.marker())
                            }
                            .as_ref()
                        );
                        assert_eq!(
                            body["messages"][1]["content"][0]["input"]["cache_control"],
                            "business data"
                        );
                    } else if with_system {
                        assert_eq!(
                            body["messages"][0]["content"].is_array(),
                            mode != PromptCache::Off
                        );
                    }
                }
            }
        }
    }
}

#[test]
#[serial]
fn cache_policy_rejects_overrides_and_unsupported_targets_before_dispatch() {
    let client = provider("ANTHROPIC");
    for key in ["cache_control", "system", "messages", "tools", "model"] {
        let mut opts = options(PromptCache::Off, CacheTtl::FiveMinutes);
        opts.extra.insert(key.into(), json!({"type":"ephemeral"}));
        assert!(client
            .input_token_reservation(&Conversation::new(), &opts)
            .is_err());
    }
    let client = provider("OPENROUTER");
    let mut opts = options(PromptCache::Rolling, CacheTtl::FiveMinutes);
    for model in ["another/model", "prefix/anthropic/fixture"] {
        opts.model = Some(model.into());
        assert!(client
            .build_request_body(&Conversation::new(), &opts)
            .is_err());
    }
    opts.model = Some("~anthropic/fixture".into());
    assert!(client
        .build_request_body(&Conversation::new(), &opts)
        .is_ok());
    assert!(provider("OPENAI")
        .build_request_body(
            &Conversation::new(),
            &options(PromptCache::Rolling, CacheTtl::FiveMinutes)
        )
        .is_err());
}

#[test]
#[serial]
fn prepared_cache_policy_is_idempotent_and_changes_contract_and_provider_identity() {
    let plain = provider("ANTHROPIC");
    let before = plain.configuration_identity();
    let cached = plain.with_prompt_cache(PromptCacheConfig {
        mode: PromptCache::Rolling,
        ttl: CacheTtl::FiveMinutes,
    });
    assert_ne!(before, cached.configuration_identity());
    let mut options = InferenceOptions::default();
    let original = crate::reasoning::prepared::digest_json(&json!(options)).unwrap();
    cached.prepare_options(&mut options).unwrap();
    let prepared = json!(options);
    cached.prepare_options(&mut options).unwrap();
    assert_eq!(prepared, json!(options));
    assert_ne!(
        original,
        crate::reasoning::prepared::digest_json(&json!(options)).unwrap()
    );
}

#[test]
#[serial]
fn rolling_conversation_prefix_stays_stable_and_empty_turns_get_no_explicit_marker() {
    let client = provider("ANTHROPIC");
    let opts = options(PromptCache::Rolling, CacheTtl::FiveMinutes);
    let mut conv = Conversation::with_system("Stable policy");
    conv.push(ConversationMessage::user("First turn"));
    let first = client.build_request_body(&conv, &opts).unwrap();
    conv.push(ConversationMessage::assistant("Acknowledged"));
    conv.push(ConversationMessage::user(""));
    let second = client.build_request_body(&conv, &opts).unwrap();
    assert_eq!(first["messages"][0], second["messages"][0]);
    assert!(second["messages"][2].get("cache_control").is_none());
}
