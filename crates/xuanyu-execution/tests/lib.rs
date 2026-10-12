use std::collections::BTreeMap;

use xuanyu_execution::{
    AgentModelSettings, ExecutionConfig, ExecutionRuntime, ModelRouter, ModelRoutingConfig,
    ModelTaskKind, ProviderConfig,
};

#[test]
fn title_task_is_tuned_for_small_json_response() {
    let config = ExecutionConfig {
        provider: ProviderConfig::openai_compatible("deepseek-chat"),
        model_routing: ModelRoutingConfig::default(),
    };
    let router = ModelRouter::new(config);
    let route = router.resolve(ModelTaskKind::Title);
    assert_eq!(route.provider.max_tokens, Some(150));
    assert_eq!(route.provider.temperature, Some(0.1));
    assert!(route.provider.extra_body.contains_key("response_format"));
    assert!(route.provider.extra_body.contains_key("thinking"));
}

#[test]
fn auxiliary_route_uses_fallback_when_distinct() {
    let mut routing = ModelRoutingConfig::default();
    routing.auxiliary_models.insert(
        ModelTaskKind::Planning,
        AgentModelSettings {
            model: Some("planner".to_string()),
            fallback: Some(Box::new(AgentModelSettings {
                model: Some("planner-fallback".to_string()),
                ..AgentModelSettings::default()
            })),
            ..AgentModelSettings::default()
        },
    );
    let router = ModelRouter::new(ExecutionConfig {
        provider: ProviderConfig::openai_compatible("main"),
        model_routing: routing,
    });
    let route = router.resolve(ModelTaskKind::Planning);
    assert_eq!(route.provider.model, "planner");
    assert_eq!(
        route.fallback.expect("fallback").provider.model,
        "planner-fallback"
    );
}

#[test]
fn execution_config_loads_from_app_config_json() {
    let config = ExecutionConfig::from_app_config_value(&serde_json::json!({
        "provider": {
            "type": "openai",
            "apiKey": "sk-test",
            "baseUrl": "https://api.example.test/v1",
            "protocol": "openai",
            "model": "main-model",
            "maxTokens": 8192,
            "temperature": 0.4,
            "fallback": { "provider": "openrouter", "model": "fallback-model" }
        },
        "modelRouting": {
            "auxiliaryModels": {
                "title": { "model": "title-model" }
            },
            "rateLimits": {
                "title": { "rpm": 6, "concurrency": 1 }
            }
        }
    }));
    let router = ModelRouter::new(config);
    let main = router.resolve(ModelTaskKind::Main);
    assert_eq!(main.provider.provider_type, "openai");
    assert_eq!(main.provider.model, "main-model");
    assert_eq!(main.provider.protocol.as_deref(), Some("openai"));
    assert_eq!(
        main.fallback.expect("fallback").provider.model,
        "fallback-model"
    );

    let title = router.resolve(ModelTaskKind::Title);
    assert_eq!(title.provider.model, "title-model");
    let rate_limit = router.resolve_rate_limit(ModelTaskKind::Title, &title);
    assert_eq!(rate_limit.rpm, Some(6));
    assert_eq!(rate_limit.concurrency, Some(1));
}

#[tokio::test]
async fn llama_cpp_route_reports_unsupported_without_network() {
    let runtime = ExecutionRuntime::new(ExecutionConfig {
        provider: ProviderConfig {
            provider_type: "llama-cpp".to_string(),
            api_key: None,
            base_url: None,
            model_path: None,
            protocol: None,
            model: "local-gguf-alias".to_string(),
            max_tokens: Some(256),
            temperature: Some(0.2),
            retry: Some(1),
            extra_body: BTreeMap::new(),
        },
        model_routing: ModelRoutingConfig::default(),
    });

    let response = runtime.generate_reply("hello").await;

    assert_eq!(response.provider, "llama-cpp");
    assert_eq!(response.model, "local-gguf-alias");
    assert!(response
        .text
        .contains("llama.cpp provider is not supported"));
}

#[tokio::test]
async fn generate_reply_reports_missing_api_key_without_network() {
    let runtime = ExecutionRuntime::new(ExecutionConfig {
        provider: ProviderConfig {
            provider_type: "openai".to_string(),
            api_key: None,
            base_url: None,
            model_path: None,
            protocol: None,
            model: "gpt-test".to_string(),
            max_tokens: Some(256),
            temperature: Some(0.2),
            retry: Some(1),
            extra_body: BTreeMap::new(),
        },
        model_routing: ModelRoutingConfig::default(),
    });

    let response = runtime.generate_reply("hello").await;

    assert_eq!(response.provider, "openai");
    assert_eq!(response.model, "gpt-test");
    assert!(response.text.contains("openai/gpt-test"));
    assert!(response.text.contains("no API key is configured"));
}

#[tokio::test]
async fn anthropic_route_reports_missing_api_key_without_network() {
    let runtime = ExecutionRuntime::new(ExecutionConfig {
        provider: ProviderConfig {
            provider_type: "anthropic".to_string(),
            api_key: None,
            base_url: None,
            model_path: None,
            protocol: Some("anthropic".to_string()),
            model: "claude-test".to_string(),
            max_tokens: Some(256),
            temperature: Some(0.2),
            retry: Some(1),
            extra_body: BTreeMap::new(),
        },
        model_routing: ModelRoutingConfig::default(),
    });

    let response = runtime.generate_reply("hello").await;

    assert_eq!(response.provider, "anthropic");
    assert_eq!(response.model, "claude-test");
    assert!(response.text.contains("anthropic/claude-test"));
    assert!(response.text.contains("no API key is configured"));
}
