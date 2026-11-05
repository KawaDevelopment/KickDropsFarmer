//! Demo-mode data: sample campaigns so the UI can be explored without a Kick account.

use std::sync::Arc;

use rand::Rng;

use super::models::{channel_preview, DropInfo, GameInfo};

pub const SPEED: f64 = 6.0;

const GAMES: [(&str, usize); 6] = [
    ("Rust", 4),
    ("Overwatch 2", 3),
    ("Counter-Strike 2", 2),
    ("Apex Legends", 3),
    ("Marvel Rivals", 2),
    ("Escape from Tarkov", 4),
];

const REQUIRED: [f64; 4] = [30.0, 60.0, 120.0, 240.0];

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' => Some(c),
            ' ' => Some('_'),
            _ => None,
        })
        .collect()
}

fn ends_at() -> String {
    (chrono::Utc::now() + chrono::Duration::days(14)).to_rfc3339()
}

pub fn games() -> Vec<GameInfo> {
    let ends = ends_at();
    GAMES
        .iter()
        .enumerate()
        .map(|(i, (name, _))| GameInfo {
            name: (*name).into(),
            category_id: Some(1000 + i as i64),
            total_drops: 0,
            unclaimed_drops: 0,
            ends_at: Some(ends.clone()),
        })
        .collect()
}

pub fn inventory() -> Vec<DropInfo> {
    let ends = ends_at();
    let mut out = Vec::new();
    for (name, count) in GAMES {
        for j in 0..count {
            let team = j + 1 == count;
            let channels = if team { vec![format!("{}_pro", slug(name))] } else { Vec::new() };
            out.push(DropInfo {
                id: format!("{}-{j}", slug(name)),
                campaign_id: Some(format!("campaign-{}", slug(name))),
                name: if team { format!("{name} Team Reward") } else { format!("{name} Drop {}", j + 1) },
                game: name.into(),
                image: None,
                required_minutes: REQUIRED[j % REQUIRED.len()],
                progress_minutes: 0.0,
                claimable: false,
                claimed: false,
                ends_at: Some(ends.clone()),
                channel_preview: channel_preview(&channels),
                channel_count: channels.len(),
                channels: Arc::new(channels),
            });
        }
    }
    out
}

pub fn channel_for(drop: &DropInfo) -> String {
    drop.channels
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{}_live", slug(&drop.game)))
}

pub fn viewers() -> u64 {
    rand::thread_rng().gen_range(800..14_000)
}
