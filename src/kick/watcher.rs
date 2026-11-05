//! The farming engine: refreshes campaigns and drop progress, farms one live
//! stream at a time, auto-claims finished drops and publishes state to the UI.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, Notify};

use super::api::{self, ChannelStatus, LiveChannel, ProgressEntry, UserStatus};
use super::demo;
use super::models::{FarmState, QueueItem, StreamSession, HISTORY_LEN};
use super::proxy::{KickProxy, WatchTarget};
use crate::notify;
use crate::settings::{self, Settings};

const TICK: Duration = Duration::from_secs(10);
const CAMPAIGN_INTERVAL: Duration = Duration::from_secs(300);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
const LIVE_INTERVAL: Duration = Duration::from_secs(60);
const PICK_RETRY: Duration = Duration::from_secs(120);
const CLAIM_COOLDOWN: Duration = Duration::from_secs(600);
const MAX_ELAPSED: Duration = Duration::from_secs(60);
const PROBE_BATCH: usize = 4;
const PROBE_LIMIT: usize = 12;
const CATEGORY_LIMIT: u32 = 24;
const QUEUE_LEN: usize = 5;

#[derive(Clone)]
struct Target {
    game: String,
    category_id: Option<i64>,
    drop_id: String,
    slug: String,
    channel_id: i64,
    livestream_id: Option<i64>,
    viewers: Option<u64>,
}

struct Candidate {
    game: String,
    category_id: Option<i64>,
    drop_id: String,
    channels: Arc<Vec<String>>,
}

impl Candidate {
    fn target(&self, slug: &str, channel_id: i64, livestream_id: Option<i64>, viewers: Option<u64>) -> Target {
        Target {
            game: self.game.clone(),
            category_id: self.category_id,
            drop_id: self.drop_id.clone(),
            slug: slug.to_string(),
            channel_id,
            livestream_id,
            viewers,
        }
    }

    fn accepts(&self, slug: &str) -> bool {
        self.channels.is_empty() || self.channels.iter().any(|s| s.eq_ignore_ascii_case(slug))
    }
}

#[derive(Default)]
struct Runtime {
    target: Option<Target>,
    last_tick: Option<Instant>,
    last_campaigns: Option<Instant>,
    last_progress: Option<Instant>,
    last_live_check: Option<Instant>,
    last_pick: Option<Instant>,
    expired: bool,
    no_live_logged: bool,
    username_tried: bool,
    tooltip: String,
}

pub struct Engine {
    app: AppHandle,
    pub state: Arc<Mutex<FarmState>>,
    settings: Arc<Mutex<Settings>>,
    pub proxy: KickProxy,
    force_refresh: AtomicBool,
    wake: Notify,
    runtime: Mutex<Runtime>,
    claim_cooldown: std::sync::Mutex<HashMap<String, Instant>>,
}

fn due(last: Option<Instant>, every: Duration) -> bool {
    last.is_none_or(|t| t.elapsed() >= every)
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn same_category(target_id: Option<i64>, target_name: &str, status: &ChannelStatus) -> bool {
    let ids_differ = matches!((target_id, status.category_id), (Some(a), Some(b)) if a != b);
    let names_differ = status
        .category_name
        .as_deref()
        .is_some_and(|n| normalize(n) != normalize(target_name));
    !(ids_differ && names_differ)
}

fn chase_indices(st: &FarmState, settings: &Settings) -> Vec<usize> {
    let mut out = Vec::new();
    for name in &settings.selected_games {
        let mut idx: Vec<usize> = st
            .inventory
            .iter()
            .enumerate()
            .filter(|(_, d)| &d.game == name && d.is_farmable())
            .map(|(i, _)| i)
            .collect();
        idx.sort_by(|a, b| {
            st.inventory[*a]
                .remaining_minutes()
                .total_cmp(&st.inventory[*b].remaining_minutes())
        });
        out.extend(idx);
    }
    out
}

fn build_queue(st: &FarmState, settings: &Settings, exclude: Option<&str>) -> Vec<QueueItem> {
    chase_indices(st, settings)
        .into_iter()
        .map(|i| &st.inventory[i])
        .filter(|d| Some(d.id.as_str()) != exclude)
        .take(QUEUE_LEN)
        .map(|d| QueueItem {
            game: d.game.clone(),
            drop_name: d.name.clone(),
            remaining_minutes: d.remaining_minutes(),
            progress_pct: d.progress_pct(),
            channel_preview: d.channel_preview.clone(),
            channel_count: d.channel_count,
        })
        .collect()
}

fn apply_progress(st: &mut FarmState, entries: &[ProgressEntry]) {
    for e in entries {
        if let Some(d) = st.inventory.iter_mut().find(|d| d.id == e.id) {
            d.progress_minutes = e.minutes;
            if e.claimed {
                d.claimed = true;
                d.claimable = false;
            } else {
                d.claimable = e.claimable.unwrap_or(false) || d.progress_minutes >= d.required_minutes;
            }
        }
    }
    st.recount_games();
}

struct SessionUpdate<'a> {
    channel: &'a str,
    channel_id: i64,
    livestream_id: Option<i64>,
    drop_id: &'a str,
    status: &'static str,
    viewers: Option<u64>,
}

fn apply_session(st: &mut FarmState, u: SessionUpdate, elapsed: Duration) -> bool {
    let Some(drop) = st.inventory.iter().find(|d| d.id == u.drop_id).cloned() else {
        st.session = None;
        return false;
    };
    let fresh = st.session.as_ref().is_none_or(|s| s.channel != u.channel);
    if fresh {
        st.session = Some(StreamSession {
            channel: u.channel.to_string(),
            channel_id: u.channel_id,
            livestream_id: u.livestream_id,
            game: drop.game.clone(),
            drop_id: String::new(),
            drop_name: String::new(),
            drop_image: None,
            progress_minutes: 0.0,
            required_minutes: 0.0,
            progress_pct: 0.0,
            watched_seconds: 0.0,
            history: VecDeque::with_capacity(HISTORY_LEN),
            status: u.status.to_string(),
            viewers: None,
        });
    }
    let watching = u.status == "watching";
    if let Some(s) = st.session.as_mut() {
        if s.drop_id != drop.id {
            s.history.clear();
        }
        s.channel_id = u.channel_id;
        s.livestream_id = u.livestream_id;
        s.game = drop.game.clone();
        s.drop_id = drop.id.clone();
        s.drop_name = drop.name.clone();
        s.drop_image = drop.image.clone();
        s.progress_minutes = drop.progress_minutes;
        s.required_minutes = drop.required_minutes;
        s.progress_pct = drop.progress_pct();
        s.status = u.status.to_string();
        if u.viewers.is_some() {
            s.viewers = u.viewers;
        }
        if watching {
            s.watched_seconds += elapsed.as_secs_f64();
        }
        let point = [s.watched_seconds / 60.0, s.progress_pct];
        if s.history.back() != Some(&point) {
            s.history.push_back(point);
            while s.history.len() > HISTORY_LEN {
                s.history.pop_front();
            }
        }
    }
    if watching {
        st.watch_seconds += elapsed.as_secs_f64();
    }
    st.idle_reason = None;
    fresh
}

fn tooltip_text(st: &FarmState) -> String {
    let line = match (&st.session, st.idle_reason) {
        (Some(s), _) => format!("Farming {} on {} ({:.0}%)", s.game, s.channel, s.progress_pct),
        (None, Some("paused")) => "Paused".into(),
        (None, Some("schedule")) => "Outside farming hours".into(),
        (None, Some("no_session")) => "Not connected to Kick".into(),
        (None, Some("expired")) => "Kick session expired".into(),
        (None, Some("no_games")) => "No games selected".into(),
        (None, Some("all_claimed")) => "All selected drops claimed".into(),
        (None, Some("no_campaigns")) => "No active campaign for your games".into(),
        (None, Some("no_live")) => "Waiting for a live stream".into(),
        _ => "Starting".into(),
    };
    format!("KickDropFarmer\n{line}").chars().take(120).collect()
}

impl Engine {
    pub fn new(app: AppHandle, settings: Arc<Mutex<Settings>>) -> Self {
        Self {
            app,
            state: Arc::new(Mutex::new(FarmState::default())),
            settings,
            proxy: KickProxy::new(),
            force_refresh: AtomicBool::new(false),
            wake: Notify::new(),
            runtime: Mutex::new(Runtime::default()),
            claim_cooldown: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn request_refresh(&self) {
        self.force_refresh.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn spawn(self: Arc<Self>) {
        tauri::async_runtime::spawn(async move {
            loop {
                self.tick().await;
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = self.wake.notified() => {}
                }
            }
        });
    }

    pub async fn publish(&self) {
        let Some(w) = self.app.get_webview_window("main") else {
            return;
        };
        if !w.is_visible().unwrap_or(false) || w.is_minimized().unwrap_or(false) {
            return;
        }
        let st = self.state.lock().await;
        let _ = self.app.emit_to("main", "farm-state", &*st);
    }

    async fn log(&self, msg: impl Into<String>) {
        self.state.lock().await.push_log(msg);
    }

    fn stop(&self, rt: &mut Runtime) {
        rt.target = None;
        self.proxy.set_watch(None);
    }

    async fn tick(&self) {
        let settings = self.settings.lock().await.clone();
        let in_schedule = settings::in_schedule(&settings);
        let paused = settings.paused || !in_schedule;
        let mut rt = self.runtime.lock().await;
        let now = Instant::now();
        let elapsed = rt
            .last_tick
            .map(|t| now.duration_since(t).min(MAX_ELAPSED))
            .unwrap_or_default();
        rt.last_tick = Some(now);

        let mode_changed = {
            let mut st = self.state.lock().await;
            let changed = st.demo != settings.demo_mode;
            if changed {
                let log = std::mem::take(&mut st.log);
                *st = FarmState {
                    log,
                    claimed_count: st.claimed_count,
                    watch_seconds: st.watch_seconds,
                    ..Default::default()
                };
            }
            st.demo = settings.demo_mode;
            st.paused = settings.paused;
            st.in_schedule = in_schedule;
            changed
        };
        if mode_changed {
            self.stop(&mut rt);
            let tooltip = std::mem::take(&mut rt.tooltip);
            *rt = Runtime {
                last_tick: Some(now),
                tooltip,
                ..Default::default()
            };
        }

        if settings.demo_mode {
            self.tick_demo(&settings, paused, elapsed).await;
        } else {
            self.tick_real(&settings, paused, elapsed, &mut rt).await;
        }
        let can_claim = settings.demo_mode || (self.proxy.has_session() && !rt.expired);
        if !paused && settings.auto_claim && can_claim {
            self.auto_claim(&settings).await;
        }

        let logs = self.proxy.drain_logs();
        let tooltip = {
            let mut st = self.state.lock().await;
            for m in logs {
                st.push_log(m);
            }
            st.last_update = Some(chrono::Local::now().format("%H:%M:%S").to_string());
            tooltip_text(&st)
        };
        if tooltip != rt.tooltip {
            if let Some(tray) = self.app.tray_by_id(crate::TRAY_ID) {
                let _ = tray.set_tooltip(Some(&tooltip));
            }
            rt.tooltip = tooltip;
        }
        drop(rt);
        self.publish().await;
    }

    // ---- demo mode ----

    async fn tick_demo(&self, settings: &Settings, paused: bool, elapsed: Duration) {
        let mut st = self.state.lock().await;
        st.logged_in = true;
        st.username = Some("demo_user".into());
        if !st.loaded {
            st.available_games = demo::games();
            st.inventory = demo::inventory();
            st.loaded = true;
            st.recount_games();
            st.push_log("Demo mode: loaded sample campaigns.");
        }

        if paused {
            st.session = None;
            st.idle_reason = Some(if settings.paused { "paused" } else { "schedule" });
            st.queue = build_queue(&st, settings, None);
            return;
        }

        let Some(&idx) = chase_indices(&st, settings).first() else {
            st.session = None;
            st.queue.clear();
            st.idle_reason = Some(if settings.selected_games.is_empty() { "no_games" } else { "all_claimed" });
            return;
        };

        let gain = elapsed.as_secs_f64() / 60.0 * demo::SPEED;
        let drop = {
            let d = &mut st.inventory[idx];
            d.progress_minutes = (d.progress_minutes + gain).min(d.required_minutes);
            d.claimable = d.progress_minutes >= d.required_minutes;
            d.clone()
        };
        let channel = demo::channel_for(&drop);
        let fresh = apply_session(
            &mut st,
            SessionUpdate {
                channel: &channel,
                channel_id: 0,
                livestream_id: None,
                drop_id: &drop.id,
                status: "watching",
                viewers: Some(demo::viewers()),
            },
            elapsed,
        );
        if fresh {
            st.push_log(format!("Now farming {}: {} on {channel}.", drop.game, drop.name));
        }
        st.recount_games();
        st.queue = build_queue(&st, settings, Some(&drop.id));
    }

    // ---- real mode ----

    async fn tick_real(&self, settings: &Settings, paused: bool, elapsed: Duration, rt: &mut Runtime) {
        let force = self.force_refresh.swap(false, Ordering::SeqCst);
        if !self.proxy.has_session() {
            self.stop(rt);
            rt.expired = false;
            let mut st = self.state.lock().await;
            st.logged_in = false;
            st.username = None;
            st.session = None;
            st.queue.clear();
            st.idle_reason = Some("no_session");
            return;
        }

        let mut repick = force;
        if force || due(rt.last_campaigns, CAMPAIGN_INTERVAL) {
            self.refresh_all(settings, rt).await;
            rt.last_campaigns = Some(Instant::now());
            rt.last_progress = rt.last_campaigns;
            repick = true;
        } else if rt.target.is_some() && due(rt.last_progress, PROGRESS_INTERVAL) {
            self.refresh_progress().await;
            rt.last_progress = Some(Instant::now());
        }

        if rt.expired || paused {
            self.stop(rt);
            let mut st = self.state.lock().await;
            st.session = None;
            st.idle_reason = Some(if rt.expired {
                "expired"
            } else if settings.paused {
                "paused"
            } else {
                "schedule"
            });
            st.queue = build_queue(&st, settings, None);
            return;
        }

        let chase = self.build_chase(settings).await;
        if chase.is_empty() {
            self.stop(rt);
            let mut st = self.state.lock().await;
            let has_campaign = settings
                .selected_games
                .iter()
                .any(|name| st.available_games.iter().any(|g| &g.name == name));
            let reason = if settings.selected_games.is_empty() {
                "no_games"
            } else if !st.loaded {
                "loading"
            } else if !has_campaign {
                "no_campaigns"
            } else {
                "all_claimed"
            };
            if st.session.is_some() && reason == "all_claimed" {
                st.push_log("All drops for your selected games are claimed or complete.");
            }
            st.session = None;
            st.queue.clear();
            st.idle_reason = Some(reason);
            return;
        }

        if let Some(t) = &rt.target {
            if !chase.iter().any(|c| c.drop_id == t.drop_id) {
                repick = true;
            }
        }

        if !repick && rt.target.is_some() && due(rt.last_live_check, LIVE_INTERVAL) {
            rt.last_live_check = Some(Instant::now());
            if let Some(t) = rt.target.clone() {
                let status = api::channel_status(&self.proxy, &t.slug).await;
                if status.reachable && !status.is_live {
                    self.log(format!("{} went offline, looking for another stream.", t.slug)).await;
                    rt.target = None;
                    repick = true;
                } else if status.is_live && !same_category(t.category_id, &t.game, &status) {
                    self.log(format!("{} switched games, looking for another stream.", t.slug)).await;
                    rt.target = None;
                    repick = true;
                } else if let Some(cur) = rt.target.as_mut() {
                    if status.viewers.is_some() {
                        cur.viewers = status.viewers;
                    }
                    if status.livestream_id.is_some() {
                        cur.livestream_id = status.livestream_id;
                    }
                }
            }
        }

        if repick || (rt.target.is_none() && due(rt.last_pick, PICK_RETRY)) {
            rt.last_pick = Some(Instant::now());
            let picked = self.pick(&chase, rt.target.as_ref()).await;
            let switched = match (&rt.target, &picked) {
                (Some(a), Some(b)) => a.slug != b.slug,
                (None, Some(_)) => true,
                _ => false,
            };
            if switched {
                rt.last_live_check = Some(Instant::now());
            }
            match &picked {
                Some(_) => rt.no_live_logged = false,
                None if !rt.no_live_logged => {
                    rt.no_live_logged = true;
                    self.log("No live streams for your unclaimed drops right now. Checking again every 2 minutes.")
                        .await;
                }
                None => {}
            }
            rt.target = picked;
        }

        let watch = rt.target.as_ref().map(|t| WatchTarget {
            channel_id: t.channel_id,
            livestream_id: t.livestream_id,
            slug: t.slug.clone(),
        });
        self.proxy.set_watch(watch.as_ref());

        let mut st = self.state.lock().await;
        match &rt.target {
            None => {
                st.session = None;
                st.idle_reason = Some("no_live");
            }
            Some(t) => {
                let status = match self.proxy.watch_state(t.channel_id) {
                    Some("watching") => "watching",
                    Some("error") => "error",
                    _ => "connecting",
                };
                let fresh = apply_session(
                    &mut st,
                    SessionUpdate {
                        channel: &t.slug,
                        channel_id: t.channel_id,
                        livestream_id: t.livestream_id,
                        drop_id: &t.drop_id,
                        status,
                        viewers: t.viewers,
                    },
                    elapsed,
                );
                if fresh {
                    let name = st.session.as_ref().map(|s| s.drop_name.clone()).unwrap_or_default();
                    st.push_log(format!("Now farming {}: {name} on {}.", t.game, t.slug));
                }
            }
        }
        st.queue = build_queue(&st, settings, rt.target.as_ref().map(|t| t.drop_id.as_str()));
    }

    async fn refresh_all(&self, settings: &Settings, rt: &mut Runtime) {
        let (mut user, campaigns, progress) = tokio::join!(
            api::fetch_user(&self.proxy),
            api::fetch_campaigns(&self.proxy),
            api::fetch_progress(&self.proxy),
        );
        let known = self.state.lock().await.username.is_some();
        if matches!(user, UserStatus::LoggedIn(None)) && !known && !rt.username_tried {
            rt.username_tried = true;
            user = UserStatus::LoggedIn(api::fetch_username(&self.proxy).await);
        }
        let mut st = self.state.lock().await;
        match user {
            UserStatus::LoggedIn(name) => {
                if rt.expired {
                    st.push_log("Kick session is valid again.");
                }
                rt.expired = false;
                st.logged_in = true;
                if name.is_some() {
                    st.username = name;
                }
            }
            UserStatus::LoggedOut => {
                if !rt.expired {
                    if st.logged_in {
                        notify::session_expiry(&self.app, settings, &self.proxy);
                    }
                    st.push_log("Kick rejected the saved session. Paste a fresh cookie header in Settings.");
                }
                rt.expired = true;
                st.logged_in = false;
            }
            UserStatus::Unknown => {}
        }

        match campaigns {
            Ok((games, mut drops)) => {
                let old: HashMap<String, (f64, bool, bool)> = st
                    .inventory
                    .drain(..)
                    .map(|d| (d.id, (d.progress_minutes, d.claimed, d.claimable)))
                    .collect();
                for d in drops.iter_mut() {
                    if let Some(&(progress, claimed, claimable)) = old.get(&d.id) {
                        d.progress_minutes = d.progress_minutes.max(progress);
                        d.claimed |= claimed;
                        d.claimable = claimable && !d.claimed;
                    }
                }
                st.inventory = drops;
                st.available_games = games;
                st.loaded = true;
            }
            Err(e) => st.push_log(format!("Could not refresh campaigns ({e}).")),
        }

        match progress {
            Ok(entries) => apply_progress(&mut st, &entries),
            Err(e) => {
                st.recount_games();
                st.push_log(format!("Could not refresh drop progress ({e})."));
            }
        }
    }

    async fn refresh_progress(&self) {
        if let Ok(entries) = api::fetch_progress(&self.proxy).await {
            apply_progress(&mut *self.state.lock().await, &entries);
        }
    }

    async fn build_chase(&self, settings: &Settings) -> Vec<Candidate> {
        let st = self.state.lock().await;
        chase_indices(&st, settings)
            .into_iter()
            .filter_map(|i| {
                let d = &st.inventory[i];
                let category_id = st
                    .available_games
                    .iter()
                    .find(|g| g.name == d.game)
                    .and_then(|g| g.category_id);
                if d.channels.is_empty() && category_id.is_none() {
                    return None;
                }
                Some(Candidate {
                    game: d.game.clone(),
                    category_id,
                    drop_id: d.id.clone(),
                    channels: d.channels.clone(),
                })
            })
            .collect()
    }

    async fn pick(&self, chase: &[Candidate], current: Option<&Target>) -> Option<Target> {
        let mut live: HashMap<i64, Vec<LiveChannel>> = HashMap::new();
        let mut probed: HashMap<String, ChannelStatus> = HashMap::new();

        for c in chase {
            if let Some(cat) = c.category_id {
                if let std::collections::hash_map::Entry::Vacant(slot) = live.entry(cat) {
                    slot.insert(api::find_live_channels(&self.proxy, cat, CATEGORY_LIMIT).await);
                }
                let list = &live[&cat];
                let sticky = current.and_then(|cur| {
                    list.iter()
                        .find(|lc| lc.slug.eq_ignore_ascii_case(&cur.slug) && c.accepts(&lc.slug))
                });
                if let Some(lc) = sticky.or_else(|| list.iter().find(|lc| c.accepts(&lc.slug))) {
                    return Some(c.target(&lc.slug, lc.channel_id, lc.livestream_id, lc.viewers));
                }
            }
            if c.channels.is_empty() {
                continue;
            }

            let mut slugs: Vec<String> = c.channels.iter().cloned().collect();
            if let Some(cur) = current {
                if let Some(pos) = slugs.iter().position(|s| s.eq_ignore_ascii_case(&cur.slug)) {
                    let s = slugs.remove(pos);
                    slugs.insert(0, s);
                }
            }
            slugs.truncate(PROBE_LIMIT);

            let pending: Vec<String> = slugs.iter().filter(|s| !probed.contains_key(*s)).cloned().collect();
            for batch in pending.chunks(PROBE_BATCH) {
                let results = join_all(batch.iter().map(|s| api::channel_status(&self.proxy, s))).await;
                for (s, r) in batch.iter().zip(results) {
                    probed.insert(s.clone(), r);
                }
                if let Some(t) = self.first_live(c, &slugs, &probed) {
                    return Some(t);
                }
            }
            if let Some(t) = self.first_live(c, &slugs, &probed) {
                return Some(t);
            }
        }
        None
    }

    fn first_live(&self, c: &Candidate, slugs: &[String], probed: &HashMap<String, ChannelStatus>) -> Option<Target> {
        slugs.iter().find_map(|s| {
            let st = probed.get(s)?;
            if !st.is_live || !same_category(c.category_id, &c.game, st) {
                return None;
            }
            let channel_id = st.channel_id?;
            Some(c.target(s, channel_id, st.livestream_id, st.viewers))
        })
    }

    // ---- claiming ----

    async fn auto_claim(&self, settings: &Settings) {
        let pending: Vec<String> = {
            let st = self.state.lock().await;
            st.inventory
                .iter()
                .filter(|d| d.claimable && !d.claimed)
                .map(|d| d.id.clone())
                .collect()
        };
        let mut claimed_any = false;
        for id in pending {
            {
                let mut cd = self.claim_cooldown.lock().unwrap();
                cd.retain(|_, t| t.elapsed() < CLAIM_COOLDOWN);
                if cd.contains_key(&id) {
                    continue;
                }
                cd.insert(id.clone(), Instant::now());
            }
            if self.claim(settings, &id).await.is_ok() {
                claimed_any = true;
            }
        }
        if claimed_any {
            self.wake();
        }
    }

    pub async fn claim_manual(&self, id: &str) -> Result<(), String> {
        let settings = self.settings.lock().await.clone();
        let result = self.claim(&settings, id).await;
        self.wake();
        result
    }

    async fn claim(&self, settings: &Settings, id: &str) -> Result<(), String> {
        let (name, game, campaign_id, demo) = {
            let st = self.state.lock().await;
            let d = st
                .inventory
                .iter()
                .find(|d| d.id == id)
                .ok_or("drop not found")?;
            if d.claimed {
                return Ok(());
            }
            (d.name.clone(), d.game.clone(), d.campaign_id.clone(), st.demo)
        };
        let result = if demo {
            Ok(())
        } else {
            api::claim_drop(&self.proxy, id, campaign_id.as_deref()).await
        };
        let mut st = self.state.lock().await;
        match &result {
            Ok(()) => {
                if let Some(d) = st.inventory.iter_mut().find(|d| d.id == id) {
                    d.claimed = true;
                    d.claimable = false;
                }
                st.claimed_count += 1;
                st.recount_games();
                st.push_log(format!("Claimed {name} ({game})."));
                notify::drop_claimed(&self.app, settings, &self.proxy, &name, &game);
            }
            Err(e) => st.push_log(format!("Could not claim {name} ({e}).")),
        }
        result
    }
}
