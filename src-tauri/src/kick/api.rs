//! Parsing layer over Kick's drops, channel and livestream responses.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::endpoints;
use super::models::{channel_preview, DropInfo, GameInfo};
use super::proxy::KickProxy;

fn as_str(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn as_f64(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_f64()))
}

fn as_i64(v: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Number(n)) => {
                if let Some(i) = n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)) {
                    return Some(i);
                }
            }
            Some(Value::String(s)) => {
                if let Ok(i) = s.parse::<i64>() {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn as_bool(v: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_bool()))
}

fn find_array<'a>(root: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    let data = root.get("data").unwrap_or(&Value::Null);
    for base in [root, data] {
        for k in keys {
            if let Some(arr) = base.get(*k).and_then(|x| x.as_array()) {
                return arr.iter().collect();
            }
        }
    }
    root.as_array()
        .or_else(|| data.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default()
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

pub enum UserStatus {
    LoggedIn(Option<String>),
    LoggedOut,
    Unknown,
}

fn find_username(body: &str) -> Option<String> {
    const KEYS: [&str; 5] = ["username", "user_name", "display_name", "slug", "name"];
    let v: Value = serde_json::from_str(body).ok()?;
    let data = v.get("data").unwrap_or(&Value::Null);
    let name = [&v, data]
        .into_iter()
        .flat_map(|base| [Some(base), base.get("user"), base.get("profile"), base.get("channel")])
        .flatten()
        .find_map(|obj| as_str(obj, &KEYS));
    name
}

pub async fn fetch_user(proxy: &KickProxy) -> UserStatus {
    match proxy.get_json(endpoints::USER).await {
        Ok((200, body)) => UserStatus::LoggedIn(find_username(&body)),
        Ok((400 | 401 | 419, _)) => UserStatus::LoggedOut,
        _ => UserStatus::Unknown,
    }
}

pub async fn fetch_username(proxy: &KickProxy) -> Option<String> {
    match proxy.get_json(endpoints::ACCOUNT).await {
        Ok((200, body)) => find_username(&body),
        _ => None,
    }
}

pub async fn fetch_campaigns(proxy: &KickProxy) -> Result<(Vec<GameInfo>, Vec<DropInfo>), String> {
    let (status, body) = proxy.get_json(endpoints::CAMPAIGNS).await?;
    if status != 200 {
        return Err(format!("campaigns HTTP {status}"));
    }
    let root: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now();

    let mut games: Vec<GameInfo> = Vec::new();
    let mut game_index: HashMap<String, usize> = HashMap::new();
    let mut drops: Vec<DropInfo> = Vec::new();

    for c in find_array(&root, &["campaigns", "data"]) {
        let ends_at = as_str(c, &["ends_at", "end_at", "expires_at", "end_date", "finished_at"]);
        if ends_at.as_deref().and_then(parse_time).is_some_and(|end| end < now) {
            continue;
        }
        let starts_at = as_str(c, &["starts_at", "start_at", "start_date", "begins_at"]);
        if starts_at.as_deref().and_then(parse_time).is_some_and(|start| start > now) {
            continue;
        }

        let category = c.get("category").or_else(|| c.get("subcategory"));
        let game_name = category
            .and_then(|cat| as_str(cat, &["name"]))
            .or_else(|| as_str(c, &["game", "category_name", "game_name", "name"]))
            .unwrap_or_else(|| "Other".into());
        let category_id = category
            .and_then(|cat| as_i64(cat, &["id", "category_id"]))
            .or_else(|| as_i64(c, &["category_id"]));
        let campaign_id = as_str(c, &["id", "campaign_id", "uuid"]);
        let channels: Vec<String> = c
            .get("channels")
            .and_then(|x| x.as_array())
            .map(|chs| {
                chs.iter()
                    .filter_map(|ch| {
                        as_str(ch, &["slug", "username", "name"]).or_else(|| ch.as_str().map(String::from))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let preview = channel_preview(&channels);
        let channel_count = channels.len();
        let channels = Arc::new(channels);

        for d in find_array(c, &["drops", "rewards", "drop_rewards"]) {
            let id = as_str(d, &["reward_id", "id", "drop_id", "uuid"]).unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let required = as_f64(d, &["required_units", "required_minutes", "minutes_required", "watch_minutes", "minutes"])
                .or_else(|| as_f64(d, &["required_hours", "hours"]).map(|h| h * 60.0))
                .or_else(|| as_f64(d, &["required_seconds", "seconds"]).map(|s| s / 60.0))
                .unwrap_or(60.0);
            let claimed = as_bool(d, &["claimed", "is_claimed"]).unwrap_or(false);
            drops.push(DropInfo {
                id,
                campaign_id: campaign_id.clone(),
                name: as_str(d, &["name", "title", "reward_name"]).unwrap_or_else(|| "Drop".into()),
                game: game_name.clone(),
                image: as_str(d, &["image_url", "image", "icon", "thumbnail"])
                    .and_then(|s| endpoints::image_url(&s)),
                required_minutes: required,
                progress_minutes: 0.0,
                claimable: false,
                claimed,
                ends_at: ends_at.clone(),
                channels: channels.clone(),
                channel_preview: preview.clone(),
                channel_count,
            });
        }

        match game_index.get(&game_name) {
            Some(&i) => {
                let g = &mut games[i];
                if g.category_id.is_none() {
                    g.category_id = category_id;
                }
                let sooner = match (&g.ends_at, &ends_at) {
                    (Some(a), Some(b)) => parse_time(b) < parse_time(a),
                    (None, Some(_)) => true,
                    _ => false,
                };
                if sooner {
                    g.ends_at = ends_at.clone();
                }
            }
            None => {
                game_index.insert(game_name.clone(), games.len());
                games.push(GameInfo {
                    name: game_name,
                    category_id,
                    total_drops: 0,
                    unclaimed_drops: 0,
                    ends_at,
                });
            }
        }
    }

    games.sort_by_key(|g| g.name.to_lowercase());
    Ok((games, drops))
}

pub struct ProgressEntry {
    pub id: String,
    pub minutes: f64,
    pub claimed: bool,
    pub claimable: Option<bool>,
}

pub async fn fetch_progress(proxy: &KickProxy) -> Result<Vec<ProgressEntry>, String> {
    let (status, body) = proxy.get_json(endpoints::PROGRESS).await?;
    if status != 200 {
        return Err(format!("progress HTTP {status}"));
    }
    let root: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for camp in find_array(&root, &["data", "progress", "drops"]) {
        let units = as_f64(camp, &["progress_units", "progress_minutes", "minutes"]);
        for r in find_array(camp, &["rewards", "drops", "drop_rewards"]) {
            let Some(id) = as_str(r, &["id", "reward_id", "drop_id"]) else {
                continue;
            };
            let required = as_f64(r, &["required_units", "required_minutes"]);
            let minutes = units.or_else(|| match (as_f64(r, &["progress"]), required) {
                (Some(frac), Some(req)) => Some(frac * req),
                _ => None,
            });
            let Some(minutes) = minutes else {
                continue;
            };
            let claimed = as_bool(r, &["claimed"]).unwrap_or(false);
            let claimable = as_bool(r, &["claimable", "can_claim"])
                .or_else(|| required.map(|req| !claimed && minutes >= req));
            out.push(ProgressEntry { id, minutes, claimed, claimable });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct LiveChannel {
    pub slug: String,
    pub channel_id: i64,
    pub livestream_id: Option<i64>,
    pub viewers: Option<u64>,
}

pub async fn find_live_channels(proxy: &KickProxy, category_id: i64, limit: u32) -> Vec<LiveChannel> {
    let url = endpoints::livestreams(category_id, limit.max(1));
    let Ok((200, body)) = proxy.get_json(&url).await else {
        return Vec::new();
    };
    let root: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for item in find_array(&root, &["livestreams", "data", "channels"]) {
        let channel = item.get("channel").unwrap_or(item);
        let slug = as_str(channel, &["slug", "username", "channel_slug"])
            .or_else(|| as_str(item, &["slug", "channel_slug", "username"]));
        let channel_id = as_i64(channel, &["id", "user_id", "channel_id"]).or_else(|| as_i64(item, &["channel_id"]));
        if let (Some(slug), Some(channel_id)) = (slug, channel_id) {
            out.push(LiveChannel {
                slug,
                channel_id,
                livestream_id: as_i64(item, &["id", "livestream_id", "live_stream_id"]),
                viewers: item
                    .get("viewer_count")
                    .or_else(|| item.get("viewers"))
                    .and_then(|x| x.as_u64()),
            });
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct ChannelStatus {
    pub reachable: bool,
    pub is_live: bool,
    pub channel_id: Option<i64>,
    pub livestream_id: Option<i64>,
    pub viewers: Option<u64>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
}

pub async fn channel_status(proxy: &KickProxy, slug: &str) -> ChannelStatus {
    let mut out = ChannelStatus::default();
    let Ok((status, body)) = proxy.get_json(&endpoints::channel(slug)).await else {
        return out;
    };
    if status == 404 {
        out.reachable = true;
        return out;
    }
    if status != 200 {
        return out;
    }
    out.reachable = true;
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let data = v.get("data").unwrap_or(&v);
    out.channel_id = as_i64(data, &["id", "user_id", "channel_id"]);

    let livestream = data
        .get("livestream")
        .or_else(|| data.get("live_stream"))
        .filter(|x| !x.is_null());
    if let Some(ls) = livestream {
        out.is_live = as_bool(ls, &["is_live"]).unwrap_or(true);
        out.livestream_id = as_i64(ls, &["id", "livestream_id"]);
        out.viewers = ls
            .get("viewers")
            .or_else(|| ls.get("viewer_count"))
            .and_then(|x| x.as_u64());
        if let Some(cat) = find_array(ls, &["categories"]).first() {
            out.category_id = as_i64(cat, &["id"]);
            out.category_name = as_str(cat, &["name"]);
        }
    } else {
        out.is_live = as_bool(data, &["is_live"]).unwrap_or(false);
        out.livestream_id = as_i64(data, &["live_stream_id", "livestream_id"]);
    }
    out
}

pub async fn claim_drop(proxy: &KickProxy, reward_id: &str, campaign_id: Option<&str>) -> Result<(), String> {
    let body = serde_json::json!({
        "reward_id": reward_id,
        "campaign_id": campaign_id.unwrap_or_default(),
    });
    let (status, resp) = proxy.post_json(endpoints::CLAIM, body).await?;
    let ok_msg = serde_json::from_str::<Value>(&resp)
        .ok()
        .and_then(|v| as_str(&v, &["message"]))
        .is_some_and(|m| m.eq_ignore_ascii_case("success"));
    if ok_msg || (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("claim HTTP {status}"))
    }
}
