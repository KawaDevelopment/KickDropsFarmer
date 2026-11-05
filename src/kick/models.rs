use std::collections::VecDeque;
use std::sync::Arc;

use serde::Serialize;

pub const HISTORY_LEN: usize = 240;
const LOG_LEN: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub name: String,
    #[serde(skip)]
    pub category_id: Option<i64>,
    pub total_drops: u32,
    pub unclaimed_drops: u32,
    pub ends_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropInfo {
    pub id: String,
    #[serde(skip)]
    pub campaign_id: Option<String>,
    pub name: String,
    pub game: String,
    pub image: Option<String>,
    pub required_minutes: f64,
    pub progress_minutes: f64,
    pub claimable: bool,
    pub claimed: bool,
    pub ends_at: Option<String>,
    #[serde(skip)]
    pub channels: Arc<Vec<String>>,
    pub channel_preview: Vec<String>,
    pub channel_count: usize,
}

impl DropInfo {
    pub fn progress_pct(&self) -> f64 {
        if self.required_minutes <= 0.0 {
            return 0.0;
        }
        (self.progress_minutes / self.required_minutes * 100.0).clamp(0.0, 100.0)
    }

    pub fn remaining_minutes(&self) -> f64 {
        (self.required_minutes - self.progress_minutes).max(0.0)
    }

    pub fn is_farmable(&self) -> bool {
        !self.claimed && self.progress_minutes < self.required_minutes
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamSession {
    pub channel: String,
    #[serde(skip)]
    pub channel_id: i64,
    #[serde(skip)]
    pub livestream_id: Option<i64>,
    pub game: String,
    pub drop_id: String,
    pub drop_name: String,
    pub drop_image: Option<String>,
    pub progress_minutes: f64,
    pub required_minutes: f64,
    pub progress_pct: f64,
    pub watched_seconds: f64,
    pub history: VecDeque<[f64; 2]>,
    pub status: String,
    pub viewers: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueItem {
    pub game: String,
    pub drop_name: String,
    pub remaining_minutes: f64,
    pub progress_pct: f64,
    pub channel_preview: Vec<String>,
    pub channel_count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FarmState {
    pub logged_in: bool,
    pub username: Option<String>,
    pub demo: bool,
    pub paused: bool,
    pub in_schedule: bool,
    pub loaded: bool,
    pub idle_reason: Option<&'static str>,
    pub available_games: Vec<GameInfo>,
    pub session: Option<StreamSession>,
    pub queue: Vec<QueueItem>,
    pub inventory: Vec<DropInfo>,
    pub log: VecDeque<String>,
    pub last_update: Option<String>,
    pub claimed_count: u32,
    pub watch_seconds: f64,
}

impl FarmState {
    pub fn push_log(&mut self, msg: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.log.push_back(format!("[{ts}] {}", msg.into()));
        while self.log.len() > LOG_LEN {
            self.log.pop_front();
        }
    }

    pub fn recount_games(&mut self) {
        for g in self.available_games.iter_mut() {
            let drops = self.inventory.iter().filter(|d| d.game == g.name);
            let (mut total, mut unclaimed) = (0u32, 0u32);
            for d in drops {
                total += 1;
                if !d.claimed {
                    unclaimed += 1;
                }
            }
            g.total_drops = total;
            g.unclaimed_drops = unclaimed;
        }
    }
}

pub fn channel_preview(channels: &[String]) -> Vec<String> {
    channels.iter().take(3).cloned().collect()
}
