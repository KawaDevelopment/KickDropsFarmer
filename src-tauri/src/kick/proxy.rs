//! Kick API client and watch-time websocket manager (`KickProxy`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt, TryStreamExt};
use rand::Rng;
use serde_json::Value;
use tauri::async_runtime::JoinHandle;
use wreq::ws::message::Message;
use wreq_util::{Emulation, Platform, Profile};

use super::endpoints;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(120);
const STABLE_CONNECTION: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default)]
struct KickCookies {
    session_token: Option<String>,
    header: String,
}

#[derive(Debug, Clone)]
pub struct WatchChannel {
    pub channel_id: i64,
    pub state: &'static str,
}

#[derive(Debug, Clone)]
pub struct WatchTarget {
    pub channel_id: i64,
    pub livestream_id: Option<i64>,
    pub slug: String,
}

type WatchTasks = HashMap<i64, (JoinHandle<()>, Option<i64>)>;

#[derive(Clone)]
pub struct KickProxy {
    client: wreq::Client,
    cookies: Arc<Mutex<KickCookies>>,
    watch: Arc<Mutex<Vec<WatchChannel>>>,
    watch_tasks: Arc<Mutex<WatchTasks>>,
    logs: Arc<Mutex<Vec<String>>>,
}

impl KickProxy {
    pub fn new() -> Self {
        let emulation = Emulation::builder()
            .profile(Profile::Chrome149)
            .platform(Platform::Windows)
            .build();
        let client = wreq::Client::builder()
            .emulation(emulation)
            .build()
            .expect("failed to build Kick HTTP client");
        Self {
            client,
            cookies: Arc::new(Mutex::new(KickCookies::default())),
            watch: Arc::new(Mutex::new(Vec::new())),
            watch_tasks: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self, msg: impl Into<String>) {
        let mut l = self.logs.lock().unwrap();
        l.push(msg.into());
        let overflow = l.len().saturating_sub(100);
        if overflow > 0 {
            l.drain(..overflow);
        }
    }

    pub fn drain_logs(&self) -> Vec<String> {
        std::mem::take(&mut *self.logs.lock().unwrap())
    }

    pub fn set_manual_token(&self, token: Option<String>) {
        let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        *self.cookies.lock().unwrap() = match token {
            Some(input) if input.contains("session_token=") => KickCookies {
                session_token: cookie_value(&input, "session_token").map(|v| percent_decode(&v)),
                header: input,
            },
            Some(input) => KickCookies {
                header: format!("session_token={input}"),
                session_token: Some(input),
            },
            None => KickCookies::default(),
        };
    }

    pub fn has_session(&self) -> bool {
        self.cookies.lock().unwrap().session_token.is_some()
    }

    fn merge_set_cookies(&self, set_cookies: &[String]) {
        let updates: Vec<(String, String)> = set_cookies
            .iter()
            .filter_map(|sc| {
                let pair = sc.split(';').next()?;
                let (name, value) = pair.split_once('=')?;
                let (name, value) = (name.trim(), value.trim());
                let deleted = value.is_empty() || sc.to_ascii_lowercase().contains("max-age=0");
                if name.is_empty() || deleted {
                    return None;
                }
                Some((name.to_string(), value.to_string()))
            })
            .collect();
        if updates.is_empty() {
            return;
        }

        let mut c = self.cookies.lock().unwrap();
        if c.session_token.is_none() {
            return;
        }
        let mut pairs: Vec<(String, String)> = c
            .header
            .split(';')
            .filter_map(|p| {
                let (k, v) = p.trim().split_once('=')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        for (name, value) in updates {
            if name == "session_token" {
                c.session_token = Some(percent_decode(&value));
            }
            match pairs.iter_mut().find(|(n, _)| *n == name) {
                Some(p) => p.1 = value,
                None => pairs.push((name, value)),
            }
        }
        c.header = pairs
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
    }

    pub async fn get_json(&self, url: &str) -> Result<(u16, String), String> {
        self.request(wreq::Method::GET, url, None).await
    }

    pub async fn post_json(&self, url: &str, body: Value) -> Result<(u16, String), String> {
        self.request(wreq::Method::POST, url, Some(body)).await
    }

    async fn request(
        &self,
        method: wreq::Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<(u16, String), String> {
        let (cookie_header, bearer) = {
            let c = self.cookies.lock().unwrap();
            (c.header.clone(), c.session_token.clone())
        };

        let mut req = self
            .client
            .request(method, url)
            .header("Accept", "application/json")
            .header("X-Client-Token", endpoints::X_CLIENT_TOKEN)
            .header("X-App-Platform", "web")
            .header("Referer", "https://kick.com/")
            .header("Origin", "https://kick.com");
        if !cookie_header.is_empty() {
            req = req.header("Cookie", cookie_header);
        }
        if let Some(tok) = bearer {
            req = req.header("Authorization", format!("Bearer {tok}"));
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b.to_string());
        }

        let resp = tokio::time::timeout(REQUEST_TIMEOUT, req.send())
            .await
            .map_err(|_| "request timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect();
        let text = tokio::time::timeout(REQUEST_TIMEOUT, resp.text())
            .await
            .map_err(|_| "response timed out".to_string())?
            .map_err(|e| e.to_string())?;
        self.merge_set_cookies(&set_cookies);
        Ok((status, text))
    }

    pub async fn post_webhook(&self, url: &str, body: Value) -> Result<u16, String> {
        let req = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        let resp = tokio::time::timeout(REQUEST_TIMEOUT, req.send())
            .await
            .map_err(|_| "request timed out".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(resp.status().as_u16())
    }

    // ---- watch-time websocket manager ----

    pub fn set_watch(&self, target: Option<&WatchTarget>) {
        let want: HashSet<i64> = target.iter().map(|t| t.channel_id).collect();
        {
            let mut tasks = self.watch_tasks.lock().unwrap();
            tasks.retain(|cid, (handle, _)| {
                let keep = want.contains(cid);
                if !keep {
                    handle.abort();
                }
                keep
            });
        }
        self.watch.lock().unwrap().retain(|w| want.contains(&w.channel_id));

        let Some(t) = target else {
            return;
        };
        {
            let mut tasks = self.watch_tasks.lock().unwrap();
            if let Some((handle, lsid)) = tasks.get(&t.channel_id) {
                if t.livestream_id.is_none() || *lsid == t.livestream_id {
                    return;
                }
                handle.abort();
                tasks.remove(&t.channel_id);
            }
        }
        self.set_channel_state(t.channel_id, "connecting");
        let proxy = self.clone();
        let (cid, lsid, slug) = (t.channel_id, t.livestream_id, t.slug.clone());
        let handle = tauri::async_runtime::spawn(async move {
            watch_channel_loop(proxy, cid, lsid, slug).await;
        });
        self.watch_tasks.lock().unwrap().insert(cid, (handle, lsid));
    }

    pub fn watch_state(&self, channel_id: i64) -> Option<&'static str> {
        self.watch
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.channel_id == channel_id)
            .map(|w| w.state)
    }

    fn set_channel_state(&self, channel_id: i64, state: &'static str) {
        let mut w = self.watch.lock().unwrap();
        match w.iter_mut().find(|e| e.channel_id == channel_id) {
            Some(entry) => entry.state = state,
            None => w.push(WatchChannel { channel_id, state }),
        }
    }

    async fn get_viewer_token(&self) -> Result<String, String> {
        let (status, body) = self.get_json(endpoints::VIEWER_TOKEN).await?;
        if status != 200 {
            return Err(format!("viewer token HTTP {status}"));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let tok = v
            .get("data")
            .and_then(|d| d.get("token").and_then(|t| t.as_str()).or_else(|| d.as_str()))
            .or_else(|| v.get("token").and_then(|t| t.as_str()))
            .ok_or("no token in viewer-token response")?
            .to_string();
        Ok(tok)
    }

    async fn run_watch_ws(
        &self,
        channel_id: i64,
        livestream_id: Option<i64>,
        slug: &str,
    ) -> Result<(), String> {
        let token = self.get_viewer_token().await?;
        let url = format!("{}?token={}", endpoints::VIEWER_WS, percent_encode(&token));
        let cookie_header = self.cookies.lock().unwrap().header.clone();

        let mut builder = self
            .client
            .websocket(url.as_str())
            .header("Origin", "https://kick.com");
        if !cookie_header.is_empty() {
            builder = builder.header("Cookie", cookie_header);
        }

        let resp = tokio::time::timeout(REQUEST_TIMEOUT, builder.send())
            .await
            .map_err(|_| "websocket connect timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let ws = resp.into_websocket().await.map_err(|e| e.to_string())?;
        let (mut tx, mut rx) = ws.split();

        let handshake = serde_json::json!({
            "type": "channel_handshake",
            "data": { "message": { "channelId": channel_id } }
        })
        .to_string();
        let ping = serde_json::json!({ "type": "ping" }).to_string();
        let user_event = livestream_id.map(|lsid| {
            serde_json::json!({
                "type": "user_event",
                "data": { "message": {
                    "name": "tracking.user.watch.livestream",
                    "channel_id": channel_id,
                    "livestream_id": lsid
                }}
            })
            .to_string()
        });

        self.set_channel_state(channel_id, "watching");
        self.log(format!("{slug}: connected, crediting watch time."));

        let mut counter: u64 = 0;
        let mut last_watch = Instant::now();
        loop {
            counter += 1;
            let frame = if counter % 2 == 1 { &handshake } else { &ping };
            if tx.send(Message::text(frame.clone())).await.is_err() {
                return Err("websocket send failed".into());
            }

            if last_watch.elapsed() >= Duration::from_secs(60) {
                if let Some(ev) = &user_event {
                    if tx.send(Message::text(ev.clone())).await.is_err() {
                        return Err("watch event send failed".into());
                    }
                }
                last_watch = Instant::now();
            }

            let delay = Duration::from_secs(11 + rand::thread_rng().gen_range(2u64..=7));
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    msg = rx.try_next() => match msg {
                        Ok(Some(_)) => continue,
                        Ok(None) => return Ok(()),
                        Err(e) => return Err(e.to_string()),
                    },
                }
            }
        }
    }
}

async fn watch_channel_loop(
    proxy: KickProxy,
    channel_id: i64,
    livestream_id: Option<i64>,
    slug: String,
) {
    let mut last_err: Option<String> = None;
    let mut delay = RECONNECT_MIN;
    loop {
        proxy.set_channel_state(channel_id, "connecting");
        let started = Instant::now();
        let result = proxy.run_watch_ws(channel_id, livestream_id, &slug).await;
        if started.elapsed() >= STABLE_CONNECTION {
            delay = RECONNECT_MIN;
        }
        match result {
            Ok(()) => {
                last_err = None;
                proxy.log(format!("{slug}: connection closed, reconnecting."));
            }
            Err(e) => {
                proxy.set_channel_state(channel_id, "error");
                if last_err.as_deref() != Some(e.as_str()) {
                    proxy.log(format!("{slug}: watch failed ({e}), retrying."));
                    last_err = Some(e);
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RECONNECT_MAX);
    }
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
