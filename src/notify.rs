//! Notifications: native Windows toasts and the optional Discord webhook.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::kick::proxy::KickProxy;
use crate::settings::Settings;

fn toast(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn discord(settings: &Settings, proxy: &KickProxy, content: String) {
    if settings.discord_notifications && !settings.discord_webhook_url.is_empty() {
        crate::discord::send(proxy, settings.discord_webhook_url.clone(), content);
    }
}

pub fn drop_claimed(app: &AppHandle, settings: &Settings, proxy: &KickProxy, drop: &str, game: &str) {
    if settings.notifications_enabled && settings.notify_on_claim {
        toast(app, "Drop claimed", &format!("{drop} ({game})"));
    }
    discord(settings, proxy, format!("**Drop claimed:** {drop} ({game})"));
}

pub fn session_expiry(app: &AppHandle, settings: &Settings, proxy: &KickProxy) {
    if settings.notifications_enabled && settings.notify_on_session_expiry {
        toast(app, "Kick session expired", "Paste a fresh cookie header in Settings to keep farming.");
    }
    discord(settings, proxy, "**Kick session expired.** Paste a fresh cookie header to keep farming.".into());
}

pub fn still_running(app: &AppHandle, settings: &Settings) {
    if settings.notifications_enabled {
        toast(
            app,
            "KickDropFarmer is still running",
            "Farming continues in the system tray. Right-click the tray icon to quit.",
        );
    }
}
