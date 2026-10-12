use xuanyu_channels::{channel_credential_status, gateway_reconnect_delay, ChannelConfig};

#[test]
fn gateway_reconnect_delay_uses_exponential_backoff_with_cap() {
    assert_eq!(gateway_reconnect_delay(1).as_secs(), 5);
    assert_eq!(gateway_reconnect_delay(2).as_secs(), 10);
    assert_eq!(gateway_reconnect_delay(3).as_secs(), 20);
    assert_eq!(gateway_reconnect_delay(4).as_secs(), 40);
    assert_eq!(gateway_reconnect_delay(5).as_secs(), 60);
    assert_eq!(gateway_reconnect_delay(99).as_secs(), 60);
}

#[test]
fn discord_gateway_reports_missing_bot_token() {
    let status = channel_credential_status(&ChannelConfig {
        id: "discord-main".to_string(),
        kind: "discord".to_string(),
        enabled: true,
        token: None,
        webhook_url: None,
        transport: Some("gateway".to_string()),
        app_id: None,
        extra: None,
    });
    assert_eq!(status.status, "needs_config");
    assert!(status.missing.iter().any(|item| item.contains("Bot Token")));
}

#[test]
fn feishu_gateway_requires_app_id_and_secret() {
    let status = channel_credential_status(&ChannelConfig {
        id: "feishu-main".to_string(),
        kind: "feishu".to_string(),
        enabled: true,
        token: Some("secret".to_string()),
        webhook_url: None,
        transport: Some("gateway".to_string()),
        app_id: None,
        extra: None,
    });
    assert_eq!(status.status, "needs_config");
    assert!(status.missing.iter().any(|item| item == "appId"));
}
