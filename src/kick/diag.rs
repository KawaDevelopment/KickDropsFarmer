//! In-app diagnostics: probes each Kick endpoint with the saved session.

use serde::Serialize;
use serde_json::Value;

use super::endpoints;
use super::proxy::KickProxy;

const SAMPLE_LEN: usize = 1200;

#[derive(Debug, Clone, Serialize)]
pub struct EndpointProbe {
    pub name: String,
    pub url: String,
    pub status: u16,
    pub ok: bool,
    pub note: String,
    pub sample: String,
}

impl EndpointProbe {
    fn info(name: &str, url: &str, ok: bool, note: String) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            status: if ok { 200 } else { 0 },
            ok,
            note,
            sample: String::new(),
        }
    }
}

fn keys(v: &Value) -> String {
    v.as_object()
        .map(|m| m.keys().map(String::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

fn summarize(v: Option<&Value>, body: &str) -> String {
    match v {
        Some(Value::Object(map)) => {
            let mut note = format!("JSON object; keys: {}", keys(&Value::Object(map.clone())));
            match map.get("data") {
                Some(Value::Array(arr)) => {
                    if let Some(first) = arr.first() {
                        note.push_str(&format!(" | data[0] keys: {}", keys(first)));
                    }
                }
                Some(obj @ Value::Object(inner)) => {
                    note.push_str(&format!(" | data keys: {}", keys(obj)));
                    for (k, child) in inner.iter().filter(|(_, c)| c.is_object()) {
                        note.push_str(&format!(" | data.{k} keys: {}", keys(child)));
                    }
                }
                _ => {}
            }
            note
        }
        Some(Value::Array(arr)) => format!("JSON array; len {}", arr.len()),
        Some(_) => "JSON (non-object)".into(),
        None if body.is_empty() => "empty body".into(),
        None => "not JSON (likely HTML or a Cloudflare page)".into(),
    }
}

async fn probe(proxy: &KickProxy, name: &str, url: &str, keep_sample: bool) -> (EndpointProbe, Option<Value>) {
    match proxy.get_json(url).await {
        Ok((status, body)) => {
            let json: Option<Value> = serde_json::from_str(&body).ok();
            let probe = EndpointProbe {
                name: name.into(),
                url: url.into(),
                status,
                ok: (200..300).contains(&status),
                note: summarize(json.as_ref(), &body),
                sample: if keep_sample { body.chars().take(SAMPLE_LEN).collect() } else { String::new() },
            };
            (probe, json)
        }
        Err(e) => (
            EndpointProbe {
                name: name.into(),
                url: url.into(),
                status: 0,
                ok: false,
                note: format!("request failed: {e}"),
                sample: String::new(),
            },
            None,
        ),
    }
}

fn data_array(v: &Value) -> Vec<&Value> {
    v.get("data")
        .and_then(|d| d.as_array().or_else(|| d.get("livestreams").and_then(|x| x.as_array())))
        .or_else(|| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("?")
}

pub async fn run(proxy: &KickProxy) -> Vec<EndpointProbe> {
    let mut out = Vec::new();
    if !proxy.has_session() {
        out.push(EndpointProbe::info(
            "session",
            "(settings)",
            false,
            "No Kick cookie saved. Paste your cookie header under Settings > Kick Connection first.".into(),
        ));
        return out;
    }

    out.push(probe(proxy, "user session", endpoints::USER, false).await.0);
    out.push(probe(proxy, "account (username)", endpoints::ACCOUNT, false).await.0);

    let (camp, camp_json) = probe(proxy, "drops/campaigns", endpoints::CAMPAIGNS, false).await;
    out.push(camp);
    let mut category: Option<i64> = None;
    if let Some(v) = &camp_json {
        let campaigns = data_array(v);
        out.push(EndpointProbe::info(
            "campaign count",
            endpoints::CAMPAIGNS,
            true,
            format!("{} campaign(s) returned", campaigns.len()),
        ));
        for c in campaigns {
            let cat = c.get("category");
            if category.is_none() {
                category = cat.and_then(|x| x.get("id")).and_then(|x| x.as_i64());
            }
            let channels: Vec<&str> = c
                .get("channels")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|ch| ch.get("slug").and_then(|s| s.as_str())).collect())
                .unwrap_or_default();
            let chan_note = match channels.len() {
                0 => String::new(),
                n => format!(" | channels: {n} ({})", channels.iter().take(3).copied().collect::<Vec<_>>().join(", ")),
            };
            let rewards = c.get("rewards").and_then(|x| x.as_array()).map_or(0, |a| a.len());
            out.push(EndpointProbe::info(
                "  campaign",
                cat.map(|x| text(x, "name")).unwrap_or("?"),
                true,
                format!(
                    "{} [{}] | rewards: {rewards} | ends: {}{chan_note}",
                    text(c, "name"),
                    text(c, "status"),
                    text(c, "ends_at"),
                ),
            ));
        }
    }

    out.push(probe(proxy, "drops/progress", endpoints::PROGRESS, true).await.0);
    out.push(probe(proxy, "viewer token (websocket auth)", endpoints::VIEWER_TOKEN, false).await.0);

    if let Some(cid) = category {
        let url = endpoints::livestreams(cid, 5);
        let (ls, ls_json) = probe(proxy, "livestreams (by category)", &url, false).await;
        out.push(ls);
        let slug = ls_json.as_ref().and_then(|v| {
            data_array(v).into_iter().find_map(|item| {
                let ch = item.get("channel").unwrap_or(item);
                ch.get("slug").and_then(|s| s.as_str()).map(String::from)
            })
        });
        if let Some(slug) = slug {
            out.push(probe(proxy, "channel (live status)", &endpoints::channel(&slug), false).await.0);
        }
    }

    out
}
