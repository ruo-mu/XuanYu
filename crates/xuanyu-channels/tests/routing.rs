//! `xuanyu_channels` 公开路由/凭据 API 回归测试。
//!
//! 之前 channels 的内部测试大量依赖私有解析函数（telegram/qq/line 等）被整体删除。
//! 这里覆盖仍公开的几个纯函数：`gateway_reconnect_delay`、`channel_credential_status`、
//! `default_channel_configs`、`channel_route_map`、`channel_webhook_port_from_config`。

use serde_json::json;
use xuanyu_channels::{
    channel_credential_status, channel_route_map, channel_webhook_port_from_config,
    default_channel_configs, gateway_reconnect_delay, ChannelConfig,
};

#[test]
fn gateway_reconnect_delay_grows_exponentially_and_caps_at_60s() {
    assert_eq!(gateway_reconnect_delay(0), std::time::Duration::from_secs(5));
    assert_eq!(gateway_reconnect_delay(1), std::time::Duration::from_secs(5));
    assert_eq!(gateway_reconnect_delay(2), std::time::Duration::from_secs(10));
    assert_eq!(gateway_reconnect_delay(3), std::time::Duration::from_secs(20));
    assert_eq!(gateway_reconnect_delay(4), std::time::Duration::from_secs(40));
    assert_eq!(gateway_reconnect_delay(5), std::time::Duration::from_secs(60));
    assert_eq!(gateway_reconnect_delay(99), std::time::Duration::from_secs(60));
}

#[test]
fn default_channel_configs_returns_nonempty_list() {
    let configs = default_channel_configs();
    assert!(!configs.is_empty(), "default channel configs must not be empty");
    for cfg in &configs {
        assert!(!cfg.id.is_empty(), "channel id must not be empty");
        assert!(!cfg.kind.is_empty(), "channel kind must not be empty");
    }
}

#[test]
fn channel_route_map_routes_each_channel_to_webhook_url() {
    let configs = default_channel_configs();
    let routes = channel_route_map(&configs, 8787);
    assert!(!routes.is_empty());
    for cfg in &configs {
        let url = routes.get(&cfg.id).expect("each channel has a route");
        assert!(url.contains(&format!("/channels/webhook/{}", cfg.id)));
        assert!(url.contains("127.0.0.1:8787"));
    }
}

#[test]
fn channel_credential_status_reports_missing_for_empty_discord_config() {
    let cfg = ChannelConfig {
        id: "discord-test".into(),
        kind: "discord".into(),
        enabled: true,
        token: None,
        webhook_url: None,
        transport: Some("gateway".into()),
        app_id: None,
        extra: None,
    };
    let status = channel_credential_status(&cfg);
    assert!(!status.ready);
    assert!(!status.missing.is_empty());
}

#[test]
fn channel_credential_status_reports_ready_for_dingtalk_with_webhook_url() {
    let cfg = ChannelConfig {
        id: "dingtalk-ready".into(),
        kind: "dingtalk".into(),
        enabled: true,
        token: None,
        webhook_url: Some("https://oapi.dingtalk.com/robot/send?access_token=x".into()),
        transport: None,
        app_id: None,
        extra: None,
    };
    let status = channel_credential_status(&cfg);
    assert!(
        status.ready,
        "dingtalk webhook with webhookUrl should be ready, missing: {:?}",
        status.missing
    );
}

#[test]
fn channel_webhook_port_from_config_uses_configured_value() {
    assert_eq!(
        channel_webhook_port_from_config(&json!({ "channels": { "webhookPort": 8765 } })),
        8765
    );
}

#[test]
fn channel_webhook_port_from_config_falls_back_to_default_8787() {
    assert_eq!(channel_webhook_port_from_config(&json!({})), 8787);
}
