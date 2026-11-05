//! Every Kick.com endpoint the farmer touches, in one place.

pub const X_CLIENT_TOKEN: &str =
    "e1393935a959b4020a4491574f6490129f678acdaa92760471263db43487f823";

pub const USER: &str = "https://web.kick.com/api/v1/user/session";

pub const ACCOUNT: &str = "https://kick.com/api/v1/user";

pub const CAMPAIGNS: &str = "https://web.kick.com/api/v1/drops/campaigns";

pub const PROGRESS: &str = "https://web.kick.com/api/v1/drops/progress";

pub const CLAIM: &str = "https://web.kick.com/api/v1/drops/claim";

pub const VIEWER_TOKEN: &str = "https://websockets.kick.com/viewer/v1/token";

pub const VIEWER_WS: &str = "wss://websockets.kick.com/viewer/v1/connect";

pub const SITE: &str = "https://kick.com/";

pub fn livestreams(category_id: i64, limit: u32) -> String {
    format!(
        "https://web.kick.com/api/v1/livestreams?limit={limit}&sort=viewer_count_desc&category_id={category_id}"
    )
}

pub fn channel(slug: &str) -> String {
    format!("https://kick.com/api/v2/channels/{slug}")
}

pub fn channel_page(slug: &str) -> Option<String> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then(|| format!("https://kick.com/{slug}"))
}

pub fn image_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with("https://") {
        return Some(raw.to_string());
    }
    if raw.starts_with("http://") {
        return Some(raw.replacen("http://", "https://", 1));
    }
    if raw.starts_with("//") {
        return Some(format!("https:{raw}"));
    }
    Some(format!("https://files.kick.com/{}", raw.trim_start_matches('/')))
}
