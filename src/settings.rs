use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: String,
    pub ui_style: String,
    pub dark_mode: bool,
    pub autostart: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,

    pub notifications_enabled: bool,
    pub notify_on_claim: bool,
    pub notify_on_session_expiry: bool,

    pub auto_claim: bool,

    pub discord_notifications: bool,
    pub discord_webhook_url: String,

    pub kick_token: String,

    pub scheduler_enabled: bool,
    pub schedule_start: String,
    pub schedule_end: String,

    pub demo_mode: bool,

    pub selected_games: Vec<String>,

    pub paused: bool,

    pub window_width: f64,
    pub window_height: f64,
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "en".into(),
            ui_style: "minimal".into(),
            dark_mode: true,
            autostart: false,
            start_minimized: true,
            close_to_tray: true,
            notifications_enabled: true,
            notify_on_claim: true,
            notify_on_session_expiry: true,
            auto_claim: true,
            discord_notifications: false,
            discord_webhook_url: String::new(),
            kick_token: String::new(),
            scheduler_enabled: false,
            schedule_start: "00:00".into(),
            schedule_end: "23:59".into(),
            demo_mode: false,
            selected_games: Vec::new(),
            paused: false,
            window_width: 1100.0,
            window_height: 720.0,
            window_maximized: false,
        }
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("cannot resolve config dir")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn in_schedule(settings: &Settings) -> bool {
    if !settings.scheduler_enabled {
        return true;
    }
    let parse = |s: &str| -> Option<u32> {
        let (h, m) = s.split_once(':')?;
        Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?)
    };
    let (Some(start), Some(end)) = (parse(&settings.schedule_start), parse(&settings.schedule_end)) else {
        return true;
    };
    let now = chrono::Local::now();
    let cur = chrono::Timelike::hour(&now) * 60 + chrono::Timelike::minute(&now);
    if start <= end {
        cur >= start && cur <= end
    } else {
        cur >= start || cur <= end
    }
}
