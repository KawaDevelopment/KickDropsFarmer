//! Discord webhook sender.

use serde_json::json;

use crate::kick::proxy::KickProxy;

const HOSTS: [&str; 4] = [
    "https://discord.com/api/webhooks/",
    "https://discordapp.com/api/webhooks/",
    "https://canary.discord.com/api/webhooks/",
    "https://ptb.discord.com/api/webhooks/",
];

fn is_webhook(url: &str) -> bool {
    HOSTS.iter().any(|h| url.starts_with(h))
}

fn payload(content: &str) -> serde_json::Value {
    json!({ "username": "KickDropFarmer", "content": content })
}

pub fn send(proxy: &KickProxy, url: String, content: String) {
    if !is_webhook(url.trim()) {
        return;
    }
    let proxy = proxy.clone();
    tauri::async_runtime::spawn(async move {
        let _ = proxy.post_webhook(url.trim(), payload(&content)).await;
    });
}

pub async fn send_test(proxy: &KickProxy, url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Webhook URL is empty".into());
    }
    if !is_webhook(url) {
        return Err("That is not a Discord webhook URL".into());
    }
    let status = proxy
        .post_webhook(url, payload("KickDropFarmer test message. The webhook is working."))
        .await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("Discord returned HTTP {status}"))
    }
}
