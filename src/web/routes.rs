use super::SharedRuntimeInfo;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::{net::SocketAddr, time::{SystemTime, UNIX_EPOCH}};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    db: SqlitePool,
    oauth_client_id: String,
    oauth_client_secret: String,
    oauth_redirect_uri: String,
    bot_token: String,
    logging_identifier: String,
    timeout_days: u64,
    http: reqwest::Client,
    runtime_info: SharedRuntimeInfo,
}

#[derive(Serialize)]
struct HealthResponse { status: &'static str, database: &'static str }

#[derive(Serialize)]
struct BetaStatusResponse { beta_bot_present: bool }

const BETA_BOT_USER_ID: &str = "1532972276805537792";

#[derive(Clone, Serialize, Deserialize)]
struct DiscordGuild {
    id: String,
    name: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    owner: bool,
    #[serde(default, deserialize_with = "deserialize_permissions")]
    permissions: String,
}

fn deserialize_permissions<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PermissionValue {
        String(String),
        Number(u64),
    }

    Ok(match PermissionValue::deserialize(deserializer)? {
        PermissionValue::String(value) => value,
        PermissionValue::Number(value) => value.to_string(),
    })
}

#[derive(Serialize)]
struct UserView {
    id: String,
    username: String,
    avatar: Option<String>,
}

#[derive(Serialize)]
struct MeResponse {
    user: UserView,
    guilds: Vec<DiscordGuild>,
}

#[derive(Deserialize)]
struct OAuthCallback { code: String, state: String }

#[derive(Deserialize)]
struct OAuthToken { access_token: String }

#[derive(Deserialize)]
struct DiscordUser { id: String, username: String, avatar: Option<String> }

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct MessageTemplate {
    enabled: bool,
    content: String,
}
impl Default for MessageTemplate {
    fn default() -> Self { Self { enabled: true, content: String::new() } }
}

fn default_message_templates() -> HashMap<String, MessageTemplate> {
    HashMap::from([
        ("timeout".to_owned(), MessageTemplate { enabled: true, content: r#"Hello {user_id},

⚠️ **Security Alert**

Your account sent content commonly associated with scam campaigns.

Your account may have been compromised.

Please:

🔒 Reset your Discord password
🛡️ Enable Two-Factor Authentication
🦠 Run malware scans
🔗 Review Authorized Apps


You have been timed out in:

**{server}**

for **{duration} days**.


Detected location:

{channel}


Flagged Content:

```text
{flagged_content}
```


If this was a mistake:

{report_server}


Please include:
- The flagged message
- What you intended to send
- Any useful investigation details


-# Anti-Scam Protection System"#.to_owned() }),
        ("detection".to_owned(), MessageTemplate { enabled: true, content: r#"🚨 Scam Detected
Possible scam content detected.

User
{user}
`{user_id}`

Reason
{reason}

Flagged Content
```text
{flagged_content}
```

Channel
{channel}

Deleted
{deleted}

Timeout
{timeout_status}"#.to_owned() }),
        ("report".to_owned(), MessageTemplate { enabled: true, content: "✅ False-positive report submitted.".to_owned() }),
    ])
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct ServerSettings {
    enabled: bool,
    text_detection: bool,
    image_detection: bool,
    delete_threshold: f64,
    auto_delete: bool,
    auto_timeout: bool,
    timeout_days: u8,
    dry_run: bool,
    exempt_admins: bool,
    exempt_role_ids: Vec<String>,
    ignored_channel_ids: Vec<String>,
    logging_channel_id: Option<String>,
    #[serde(default = "default_message_templates")]
    message_templates: HashMap<String, MessageTemplate>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            text_detection: true,
            image_detection: true,
            delete_threshold: 0.805,
            auto_delete: true,
            auto_timeout: true,
            timeout_days: 7,
            dry_run: false,
            exempt_admins: true,
            exempt_role_ids: Vec::new(),
            ignored_channel_ids: Vec::new(),
            logging_channel_id: None,
            message_templates: default_message_templates(),
        }
    }
}

#[derive(Serialize)]
struct SettingsResponse {
    guild_id: String,
    settings: ServerSettings,
    updated_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(default)]
struct SettingsInput {
    enabled: bool,
    text_detection: bool,
    image_detection: bool,
    delete_threshold: f64,
    auto_delete: bool,
    auto_timeout: bool,
    timeout_days: u8,
    dry_run: bool,
    exempt_admins: bool,
    exempt_role_ids: Vec<String>,
    ignored_channel_ids: Vec<String>,
    logging_channel_id: Option<String>,
    #[serde(default = "default_message_templates")]
    message_templates: HashMap<String, MessageTemplate>,
}

impl Default for SettingsInput {
    fn default() -> Self {
        let defaults = ServerSettings::default();
        Self {
            enabled: defaults.enabled,
            text_detection: defaults.text_detection,
            image_detection: defaults.image_detection,
            delete_threshold: defaults.delete_threshold,
            auto_delete: defaults.auto_delete,
            auto_timeout: defaults.auto_timeout,
            timeout_days: defaults.timeout_days,
            dry_run: defaults.dry_run,
            exempt_admins: defaults.exempt_admins,
            exempt_role_ids: defaults.exempt_role_ids,
            ignored_channel_ids: defaults.ignored_channel_ids,
            logging_channel_id: defaults.logging_channel_id,
            message_templates: defaults.message_templates,
        }
    }
}

#[derive(Serialize)]
struct ProtectionOption {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct ProtectionOptionsResponse {
    roles: Vec<ProtectionOption>,
    channels: Vec<ProtectionOption>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

fn digest(value: &str) -> String { hex::encode(Sha256::digest(value.as_bytes())) }

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

async fn session_user(headers: &HeaderMap, db: &SqlitePool) -> Result<(String, String, Option<String>, Vec<DiscordGuild>), StatusCode> {
    let token = cookie_value(headers, "aegis_session").ok_or(StatusCode::UNAUTHORIZED)?;
    let row = sqlx::query_as::<_, (String, String, Option<String>, String)>(
        "SELECT discord_user_id, username, avatar, guilds_json FROM dashboard_sessions WHERE token_hash = ? AND expires_at > ?"
    ).bind(digest(&token)).bind(now()).fetch_optional(db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let guilds = serde_json::from_str(&row.3).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((row.0, row.1, row.2, guilds))
}

fn can_manage(guild: &DiscordGuild) -> bool {
    if guild.owner { return true; }
    guild.permissions.parse::<u64>().map(|bits| bits & (0x8 | 0x20) != 0).unwrap_or(false)
}

async fn get_runtime_info(State(state): State<AppState>) -> Json<Option<super::RuntimeInfo>> {
    Json(state.runtime_info.read().await.clone())
}

async fn beta_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(guild_id): Path<String>,
) -> Result<Json<BetaStatusResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|guild| guild.id == guild_id && can_manage(guild))
        .ok_or(StatusCode::FORBIDDEN)?;

    let response = state.http
        .get(format!("https://discord.com/api/v10/guilds/{}/members/{}", guild.id, BETA_BOT_USER_ID))
        .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token))
        .send().await.map_err(|error| {
            tracing::error!(%error, guild_id = %guild.id, "Could not check beta bot membership");
            StatusCode::BAD_GATEWAY
        })?;

    if response.status().is_success() {
        return Ok(Json(BetaStatusResponse { beta_bot_present: true }));
    }
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(Json(BetaStatusResponse { beta_bot_present: false }));
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    tracing::error!(%status, guild_id = %guild.id, response = %body.chars().take(300).collect::<String>(), "Discord beta bot membership check failed");
    Err(StatusCode::BAD_GATEWAY)
}


#[derive(Serialize)]
struct ModerationResponse {
    guild_id: String,
    logging_channel_id: Option<String>,
    lookback_days: u8,
    truncated: bool,
    warning: Option<String>,
    incidents: Vec<ModerationIncident>,
}
#[derive(Serialize)]
struct ModerationIncident {
    id: String, user_id: String, username: String, avatar_url: Option<String>,
    reason: String, flagged_content: String, confidence: Option<f64>,
    original_confidence: Option<f64>, channel_id: Option<String>, deleted: bool,
    timeout_days: u64, timeout_until: Option<String>, status: String,
    created_at: i64, log_url: String,
}
fn snowflake_timestamp(id: &str) -> Option<i64> {
    let value = id.parse::<u64>().ok()?;
    Some(((value >> 22) + 1_420_070_400_000) as i64 / 1000)
}
fn embed_field<'a>(embed: &'a Value, name: &str) -> Option<&'a str> {
    embed.get("fields")?.as_array()?.iter().find(|field| {
        field.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name))
    })?.get("value")?.as_str()
}
fn extract_id(value: &str, marker: &str) -> Option<String> {
    let after = value.split_once(marker)?.1;
    let digits: String = after.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    (!digits.is_empty()).then_some(digits)
}
fn extract_percent(value: &str, label: &str) -> Option<f64> {
    let after = value.split_once(label)?.1.trim_start();
    let number: String = after.chars().take_while(|ch| ch.is_ascii_digit() || *ch == '.').collect();
    number.parse().ok()
}
fn clean_code_block(value: &str) -> String {
    let fence = char::from(96).to_string().repeat(3);
    let prefix = format!("{fence}text");
    let trimmed = value.trim();
    let without_start = trimmed.strip_prefix(&prefix).unwrap_or(trimmed).trim();
    without_start.strip_suffix(&fence).unwrap_or(without_start).trim().to_owned()
}
async fn moderation(
    State(state): State<AppState>, headers: HeaderMap, Path(guild_id): Path<String>,
) -> Result<Json<ModerationResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|g| g.id == guild_id && can_manage(g)).ok_or(StatusCode::FORBIDDEN)?;
    let runtime = state.runtime_info.read().await.clone().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings_json = sqlx::query_scalar::<_, String>("SELECT config_json FROM guild_configs WHERE guild_id = ?")
        .bind(&guild.id).fetch_optional(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let parsed_settings = settings_json.and_then(|json| serde_json::from_str::<ServerSettings>(&json).ok());
    let configured_channel = parsed_settings.as_ref().and_then(|settings| settings.logging_channel_id.clone());
    let guild_timeout_days = parsed_settings.map(|settings| settings.timeout_days as u64).unwrap_or(state.timeout_days).clamp(1, 7);
    let channels_response = state.http.get(format!("https://discord.com/api/v10/guilds/{}/channels", guild.id))
        .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token)).send().await
        .map_err(|error| { tracing::error!(%error, guild_id = %guild.id, "Could not fetch Discord server channels for moderation history"); StatusCode::BAD_GATEWAY })?;
    if !channels_response.status().is_success() {
        let status = channels_response.status();
        let body = channels_response.text().await.unwrap_or_default();
        tracing::error!(%status, guild_id = %guild.id, response = %body.chars().take(300).collect::<String>(), "Discord rejected moderation channel lookup");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let channels: Vec<Value> = channels_response.json().await.map_err(|error| {
        tracing::error!(%error, guild_id = %guild.id, "Could not parse Discord channel list"); StatusCode::BAD_GATEWAY
    })?;
    let log_channel = configured_channel.as_ref()
        .and_then(|id| channels.iter().find(|channel| channel.get("id").and_then(Value::as_str) == Some(id)))
        .or_else(|| channels.iter().find(|channel| {
            channel.get("topic").and_then(Value::as_str)
                .is_some_and(|topic| topic.contains(&state.logging_identifier))
        }));
    let Some(log_channel_id) = log_channel.and_then(|c| c.get("id").and_then(Value::as_str)).map(str::to_owned) else {
        return Ok(Json(ModerationResponse { guild_id, logging_channel_id: None, lookback_days: 9, truncated: false,
            warning: Some("Aegis could not find this server's logging channel. Configure the Aegis logging channel first.".to_owned()),
            incidents: Vec::new() }));
    };
    let cutoff = now() - 9 * 86_400;
    let mut before: Option<String> = None;
    let mut raw: Vec<(String,String,String,String,Option<f64>,Option<f64>,Option<String>,bool,i64)> = Vec::new();
    let mut reached_cutoff = false;
    for _ in 0..100 {
        let url = format!("https://discord.com/api/v10/channels/{}/messages", log_channel_id);
        let mut request = state.http.get(url).header(header::AUTHORIZATION, format!("Bot {}", state.bot_token));
        request = if let Some(ref cursor) = before {
            request.query(&[("limit", "100"), ("before", cursor.as_str())])
        } else { request.query(&[("limit", "100")]) };
        let response = request.send().await.map_err(|error| {
            tracing::error!(%error, guild_id = %guild.id, channel_id = %log_channel_id, "Could not fetch Aegis moderation log history");
            StatusCode::BAD_GATEWAY
        })?;
        if !response.status().is_success() {
            let status = response.status(); let body = response.text().await.unwrap_or_default();
            tracing::error!(%status, guild_id = %guild.id, channel_id = %log_channel_id, response = %body.chars().take(300).collect::<String>(), "Discord rejected moderation log history request");
            return Err(StatusCode::BAD_GATEWAY);
        }
        let messages: Vec<Value> = response.json().await.map_err(|error| {
            tracing::error!(%error, guild_id = %guild.id, "Could not parse moderation log messages"); StatusCode::BAD_GATEWAY
        })?;
        if messages.is_empty() { reached_cutoff = true; break; }
        let oldest_id = messages.last().and_then(|m| m.get("id")).and_then(Value::as_str).map(str::to_owned);
        for message in &messages {
            let message_id = message.get("id").and_then(Value::as_str).unwrap_or_default();
            let Some(created_at) = snowflake_timestamp(message_id) else { continue; };
            if created_at < cutoff { reached_cutoff = true; continue; }
            if message.get("author").and_then(|a| a.get("id")).and_then(Value::as_str) != Some(runtime.bot_user_id.as_str()) { continue; }
            let Some(embeds) = message.get("embeds").and_then(Value::as_array) else { continue; };
            let Some(embed) = embeds.iter().find(|e| e.get("title").and_then(Value::as_str).is_some_and(|t| t.contains("Scam Detected"))) else { continue; };
            let Some(timeout) = embed_field(embed, "Timeout") else { continue; };
            if !timeout.contains("Timed out successfully") { continue; }
            let Some(user_field) = embed_field(embed, "User") else { continue; };
            let Some(user_id) = user_field.split(char::from(96)).nth(1).map(str::to_owned).or_else(|| extract_id(user_field, "<@")) else { continue; };
            let reason = embed_field(embed, "Reason").unwrap_or("Unknown detection reason").to_owned();
            let flagged = clean_code_block(embed_field(embed, "Flagged Content").unwrap_or("No flagged content was included in the log."));
            let confidence = extract_percent(&flagged, "Confidence:");
            let original_confidence = extract_percent(&flagged, "Original Confidence:");
            let channel_id = embed_field(embed, "Channel").and_then(|v| extract_id(v, "<#"));
            let deleted = embed_field(embed, "Deleted").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
            raw.push((message_id.to_owned(), user_id, reason, flagged, confidence, original_confidence, channel_id, deleted, created_at));
        }
        let Some(oldest_id) = oldest_id else { reached_cutoff = true; break; };
        if snowflake_timestamp(&oldest_id).is_some_and(|t| t < cutoff) || messages.len() < 100 {
            reached_cutoff = true; break;
        }
        before = Some(oldest_id);
    }
    let truncated = !reached_cutoff && before.is_some();
    raw.sort_by(|a, b| b.8.cmp(&a.8));
    let mut member_cache: HashMap<String,(String,Option<String>,Option<String>,bool)> = HashMap::new();
    let mut active_assigned: HashSet<String> = HashSet::new();
    let mut incidents = Vec::with_capacity(raw.len());
    for (message_id,user_id,reason,flagged_content,confidence,original_confidence,channel_id,deleted,created_at) in raw {
        if !member_cache.contains_key(&user_id) {
            let response = state.http.get(format!("https://discord.com/api/v10/guilds/{}/members/{}", guild.id, user_id))
                .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token)).send().await
                .map_err(|error| { tracing::error!(%error, guild_id = %guild.id, user_id = %user_id, "Could not fetch member details for moderation history"); StatusCode::BAD_GATEWAY })?;
            if response.status().is_success() {
                let member: Value = response.json().await.map_err(|error| {
                    tracing::error!(%error, user_id = %user_id, "Could not parse Discord member details"); StatusCode::BAD_GATEWAY
                })?;
                let user = member.get("user").unwrap_or(&Value::Null);
                let username = user.get("global_name").and_then(Value::as_str).or_else(|| user.get("username").and_then(Value::as_str)).unwrap_or(&user_id).to_owned();
                let avatar = member.get("avatar").and_then(Value::as_str)
                    .map(|hash| format!("https://cdn.discordapp.com/guilds/{}/users/{}/avatars/{}.png?size=64", guild.id, user_id, hash))
                    .or_else(|| user.get("avatar").and_then(Value::as_str).map(|hash| format!("https://cdn.discordapp.com/avatars/{}/{}.png?size=64", user_id, hash)));
                let timeout = member.get("communication_disabled_until").and_then(Value::as_str).map(str::to_owned);
                member_cache.insert(user_id.clone(), (username, avatar, timeout, true));
            } else if response.status() == StatusCode::NOT_FOUND {
                member_cache.insert(user_id.clone(), (user_id.clone(), None, None, false));
            } else {
                let status = response.status(); let body = response.text().await.unwrap_or_default();
                tracing::error!(%status, guild_id = %guild.id, user_id = %user_id, response = %body.chars().take(300).collect::<String>(), "Discord rejected member lookup for moderation history");
                return Err(StatusCode::BAD_GATEWAY);
            }
        }
        let (username,avatar_url,live_timeout,member_present) = member_cache.get(&user_id).cloned().unwrap_or_else(|| (user_id.clone(),None,None,false));
        let expected_until = created_at.saturating_add((guild_timeout_days.min(i64::MAX as u64 / 86_400)) as i64 * 86_400);
        let currently_timed_out = live_timeout.is_some();
        let status = (if !member_present { "unknown" } else if currently_timed_out && active_assigned.insert(user_id.clone()) { "active" } else if now() < expected_until { "ended_early" } else { "expired" }).to_owned();
        let timeout_until = if status == "active" { live_timeout } else { Some(expected_until.to_string()) };
        let log_url = format!("https://discord.com/channels/{}/{}/{}", guild.id, log_channel_id, message_id);
        incidents.push(ModerationIncident { id: message_id, user_id, username, avatar_url, reason, flagged_content,
            confidence, original_confidence, channel_id, deleted, timeout_days: guild_timeout_days,
            timeout_until, status, created_at, log_url });
    }
    incidents.sort_by(|a,b| b.created_at.cmp(&a.created_at));
    Ok(Json(ModerationResponse { guild_id, logging_channel_id: Some(log_channel_id), lookback_days: 9,
        truncated, warning: None, incidents }))
}

async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, StatusCode> {
    sqlx::query("SELECT 1").execute(&state.db).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(HealthResponse { status: "ok", database: "ok" }))
}

async fn login(State(state): State<AppState>) -> Result<Response, StatusCode> {
    if state.oauth_client_id.is_empty() || state.oauth_client_secret.is_empty() || state.oauth_redirect_uri.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let state_token = random_token();
    sqlx::query("DELETE FROM dashboard_oauth_states WHERE expires_at <= ?").bind(now()).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("INSERT INTO dashboard_oauth_states(state_hash, expires_at) VALUES(?, ?)")
        .bind(digest(&state_token)).bind(now() + 600).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut url = url::Url::parse("https://discord.com/oauth2/authorize").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.oauth_client_id)
        .append_pair("redirect_uri", &state.oauth_redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", "identify guilds")
        .append_pair("state", &state_token);
    let mut response = Redirect::temporary(url.as_str()).into_response();
    let cookie = format!("aegis_oauth_state={}; Path=/auth/callback; HttpOnly; SameSite=Lax; Max-Age=600", state_token);
    response.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    Ok(response)
}

async fn oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallback>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let browser_state = cookie_value(&headers, "aegis_oauth_state").ok_or(StatusCode::BAD_REQUEST)?;
    if browser_state != query.state { return Err(StatusCode::BAD_REQUEST); }
    let valid = sqlx::query("DELETE FROM dashboard_oauth_states WHERE state_hash = ? AND expires_at > ?")
        .bind(digest(&query.state)).bind(now()).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.rows_affected() == 1;
    if !valid { return Err(StatusCode::BAD_REQUEST); }

    let token_response = state.http.post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", state.oauth_client_id.as_str()),
            ("client_secret", state.oauth_client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", query.code.as_str()),
            ("redirect_uri", state.oauth_redirect_uri.as_str()),
        ])
        .send().await.map_err(|error| {
            tracing::error!(%error, "Discord OAuth token request failed");
            StatusCode::BAD_GATEWAY
        })?;
    let token_status = token_response.status();
    if !token_status.is_success() {
        let body = token_response.text().await.unwrap_or_default();
        tracing::error!(status = %token_status, response = %body.chars().take(500).collect::<String>(), "Discord rejected OAuth token exchange");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let token: OAuthToken = token_response.json().await.map_err(|error| {
        tracing::error!(%error, "Could not parse Discord OAuth token response");
        StatusCode::BAD_GATEWAY
    })?;

    let user_response = state.http.get("https://discord.com/api/users/@me")
        .bearer_auth(&token.access_token).send().await.map_err(|error| {
            tracing::error!(%error, "Discord current-user request failed");
            StatusCode::BAD_GATEWAY
        })?;
    let user_status = user_response.status();
    if !user_status.is_success() {
        let body = user_response.text().await.unwrap_or_default();
        tracing::error!(status = %user_status, response = %body.chars().take(500).collect::<String>(), "Discord rejected current-user request");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let user: DiscordUser = user_response.json().await.map_err(|error| {
        tracing::error!(%error, "Could not parse Discord current-user response");
        StatusCode::BAD_GATEWAY
    })?;

    let guilds_response = state.http.get("https://discord.com/api/users/@me/guilds")
        .bearer_auth(&token.access_token).send().await.map_err(|error| {
            tracing::error!(%error, "Discord guild-list request failed");
            StatusCode::BAD_GATEWAY
        })?;
    let guilds_status = guilds_response.status();
    if !guilds_status.is_success() {
        let body = guilds_response.text().await.unwrap_or_default();
        tracing::error!(status = %guilds_status, response = %body.chars().take(500).collect::<String>(), "Discord rejected guild-list request");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let guilds_body = guilds_response.bytes().await.map_err(|error| {
        tracing::error!(%error, "Could not read Discord guild-list response body");
        StatusCode::BAD_GATEWAY
    })?;
    let guilds: Vec<DiscordGuild> = serde_json::from_slice(&guilds_body).map_err(|error| {
        tracing::error!(%error, "Could not parse Discord guild-list response");
        StatusCode::BAD_GATEWAY
    })?;

    let session = random_token();
    let avatar = user.avatar.clone();
    let guilds_json = serde_json::to_string(&guilds).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("INSERT INTO dashboard_sessions(token_hash, discord_user_id, expires_at, created_at, username, avatar, guilds_json) VALUES(?, ?, ?, ?, ?, ?, ?)")
        .bind(digest(&session)).bind(&user.id).bind(now() + 60 * 60).bind(now())
        .bind(&user.username).bind(&avatar).bind(guilds_json).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut response = Redirect::to("/#overview").into_response();
    let cookie = format!("aegis_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=3600", session);
    response.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    response.headers_mut().append(header::SET_COOKIE, HeaderValue::from_static("aegis_oauth_state=; Path=/auth/callback; HttpOnly; SameSite=Lax; Max-Age=0"));
    Ok(response)
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<MeResponse>, StatusCode> {
    let (id, username, avatar, guilds) = session_user(&headers, &state.db).await?;
    Ok(Json(MeResponse { user: UserView { id, username, avatar }, guilds }))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    if let Some(token) = cookie_value(&headers, "aegis_session") {
        sqlx::query("DELETE FROM dashboard_sessions WHERE token_hash = ?")
            .bind(digest(&token)).execute(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_static("aegis_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"));
    Ok(response)
}

async fn protection_options(
    State(state): State<AppState>, headers: HeaderMap, Path(guild_id): Path<String>,
) -> Result<Json<ProtectionOptionsResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|g| g.id == guild_id && can_manage(g)).ok_or(StatusCode::FORBIDDEN)?;

    let roles_response = state.http
        .get(format!("https://discord.com/api/v10/guilds/{}/roles", guild.id))
        .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token))
        .send().await.map_err(|error| {
            tracing::error!(%error, guild_id = %guild.id, "Could not fetch Discord roles for protection settings");
            StatusCode::BAD_GATEWAY
        })?;
    if !roles_response.status().is_success() {
        let status = roles_response.status();
        let body = roles_response.text().await.unwrap_or_default();
        tracing::error!(%status, guild_id = %guild.id, response = %body.chars().take(300).collect::<String>(), "Discord rejected protection role lookup");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let role_values: Vec<Value> = roles_response.json().await.map_err(|error| {
        tracing::error!(%error, guild_id = %guild.id, "Could not parse Discord roles");
        StatusCode::BAD_GATEWAY
    })?;
    let roles = role_values.into_iter().filter_map(|role| {
        let id = role.get("id")?.as_str()?.to_owned();
        if id == guild.id { return None; }
        Some(ProtectionOption {
            id,
            name: role.get("name").and_then(Value::as_str).unwrap_or("Unnamed role").to_owned(),
        })
    }).collect();

    let channels_response = state.http
        .get(format!("https://discord.com/api/v10/guilds/{}/channels", guild.id))
        .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token))
        .send().await.map_err(|error| {
            tracing::error!(%error, guild_id = %guild.id, "Could not fetch Discord channels for protection settings");
            StatusCode::BAD_GATEWAY
        })?;
    if !channels_response.status().is_success() {
        let status = channels_response.status();
        let body = channels_response.text().await.unwrap_or_default();
        tracing::error!(%status, guild_id = %guild.id, response = %body.chars().take(300).collect::<String>(), "Discord rejected protection channel lookup");
        return Err(StatusCode::BAD_GATEWAY);
    }
    let channel_values: Vec<Value> = channels_response.json().await.map_err(|error| {
        tracing::error!(%error, guild_id = %guild.id, "Could not parse Discord channels");
        StatusCode::BAD_GATEWAY
    })?;
    let channels = channel_values.into_iter().filter_map(|channel| {
        let kind = channel.get("type").and_then(Value::as_u64)?;
        if kind != 0 && kind != 5 { return None; }
        Some(ProtectionOption {
            id: channel.get("id")?.as_str()?.to_owned(),
            name: channel.get("name").and_then(Value::as_str).unwrap_or("unnamed-channel").to_owned(),
        })
    }).collect();

    Ok(Json(ProtectionOptionsResponse { roles, channels }))
}

async fn get_settings(
    State(state): State<AppState>, headers: HeaderMap, Path(guild_id): Path<String>,
) -> Result<Json<SettingsResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|g| g.id == guild_id && can_manage(g)).ok_or(StatusCode::FORBIDDEN)?;
    let row = sqlx::query_as::<_, (String, i64)>("SELECT config_json, updated_at FROM guild_configs WHERE guild_id = ?")
        .bind(&guild.id).fetch_optional(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (settings, updated_at) = match row {
        Some((json, updated)) => (serde_json::from_str(&json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?, Some(updated)),
        None => (ServerSettings::default(), None),
    };
    Ok(Json(SettingsResponse { guild_id, settings, updated_at }))
}

async fn save_settings(
    State(state): State<AppState>, headers: HeaderMap, Path(guild_id): Path<String>, Json(input): Json<SettingsInput>,
) -> Result<Json<SettingsResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|g| g.id == guild_id && can_manage(g)).ok_or(StatusCode::FORBIDDEN)?;
    if !(1..=7).contains(&input.timeout_days) || !input.delete_threshold.is_finite()
        || !(0.5..=1.0).contains(&input.delete_threshold) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let logging_channel_id = input.logging_channel_id.filter(|v| !v.trim().is_empty());
    let previous_json = sqlx::query_as::<_, (String,)>("SELECT config_json FROM guild_configs WHERE guild_id = ?")
        .bind(&guild.id).fetch_optional(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let previous_channel = previous_json.and_then(|(json,)| serde_json::from_str::<Value>(&json).ok())
        .and_then(|value| value.get("logging_channel_id").and_then(Value::as_str).map(str::to_owned));
    if logging_channel_id != previous_channel {
        if let Some(channel_id) = logging_channel_id.as_deref() {
            let response = state.http.get(format!("https://discord.com/api/v10/guilds/{}/channels", guild.id))
                .header(header::AUTHORIZATION, format!("Bot {}", state.bot_token))
                .send().await.map_err(|error| {
                    tracing::error!(%error, guild_id = %guild.id, "Could not validate selected logging channel");
                    StatusCode::BAD_GATEWAY
                })?;
            if !response.status().is_success() { return Err(StatusCode::BAD_GATEWAY); }
            let channels: Vec<Value> = response.json().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
            let valid_channel = channels.iter().any(|channel| {
                channel.get("id").and_then(Value::as_str) == Some(channel_id)
                    && matches!(channel.get("type").and_then(Value::as_u64), Some(0 | 5))
            });
            if !valid_channel { return Err(StatusCode::UNPROCESSABLE_ENTITY); }
        }
    }
    let valid_ids = |ids: &[String]| ids.len() <= 250 && ids.iter().all(|id| !id.is_empty() && id.len() <= 25 && id.chars().all(|c| c.is_ascii_digit()));
    if logging_channel_id.as_ref().is_some_and(|v| !v.chars().all(|c| c.is_ascii_digit()) || v.len() > 25)
        || !valid_ids(&input.exempt_role_ids) || !valid_ids(&input.ignored_channel_ids) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let mut message_templates = default_message_templates();
    for key in ["timeout", "detection", "report"] {
        if let Some(template) = input.message_templates.get(key) {
            if template.content.len() > 5000 { return Err(StatusCode::UNPROCESSABLE_ENTITY); }
            message_templates.insert(key.to_owned(), template.clone());
        }
    }
    let settings = ServerSettings {
        enabled: input.enabled,
        text_detection: input.text_detection,
        image_detection: input.image_detection,
        delete_threshold: input.delete_threshold,
        auto_delete: input.auto_delete,
        auto_timeout: input.auto_timeout,
        timeout_days: input.timeout_days,
        dry_run: input.dry_run,
        exempt_admins: input.exempt_admins,
        exempt_role_ids: input.exempt_role_ids,
        ignored_channel_ids: input.ignored_channel_ids,
        logging_channel_id,
        message_templates,
    };
    let json = serde_json::to_string(&settings).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let updated = now();
    sqlx::query("INSERT INTO guild_configs(guild_id, config_json, updated_at) VALUES(?, ?, ?) ON CONFLICT(guild_id) DO UPDATE SET config_json=excluded.config_json, updated_at=excluded.updated_at")
        .bind(&guild.id).bind(json).bind(updated).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SettingsResponse { guild_id, settings, updated_at: Some(updated) }))
}


#[derive(Deserialize)]
struct AnalyticsQuery { range: Option<String> }

#[derive(Serialize)]
struct AnalyticsBucket {
    timestamp: i64,
    messages: i64,
    scam_detections: i64,
    average_confidence: Option<f64>,
    deleted_messages: i64,
    timeouts: i64,
}

#[derive(Serialize)]
struct AnalyticsResponse {
    guild_id: String,
    range: String,
    timezone: &'static str,
    totals: AnalyticsBucket,
    buckets: Vec<AnalyticsBucket>,
}

async fn analytics(
    State(state): State<AppState>, headers: HeaderMap, Path(guild_id): Path<String>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<AnalyticsResponse>, StatusCode> {
    let (_, _, _, guilds) = session_user(&headers, &state.db).await?;
    let guild = guilds.iter().find(|g| g.id == guild_id && can_manage(g)).ok_or(StatusCode::FORBIDDEN)?;
    let range = query.range.as_deref().unwrap_or("week");
    let (periods, bucket_seconds) = match range {
        "day" => (24_i64, 3_600_i64),
        "week" => (7_i64, 86_400_i64),
        "month" => (30_i64, 86_400_i64),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let current = now();
    // Anchor the window to the current bucket so the latest hour/day is included.
    let start = current.div_euclid(bucket_seconds) * bucket_seconds - (periods - 1) * bucket_seconds;
    let rows = sqlx::query("SELECT hour_start, messages, scam_detections, confidence_sum, confidence_count, deleted_messages, timeouts FROM analytics_hourly WHERE guild_id = ? AND hour_start >= ? ORDER BY hour_start")
        .bind(&guild.id).bind(start).fetch_all(&state.db).await
        .map_err(|error| { tracing::error!(%error, guild_id = %guild.id, "Could not load analytics aggregates"); StatusCode::INTERNAL_SERVER_ERROR })?;
    let mut values: HashMap<i64, (i64,i64,f64,i64,i64,i64)> = HashMap::new();
    for row in rows {
        let hour: i64 = row.try_get("hour_start").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let bucket = hour.div_euclid(bucket_seconds) * bucket_seconds;
        let entry = values.entry(bucket).or_insert((0,0,0.0,0,0,0));
        entry.0 += row.try_get::<i64,_>("messages").unwrap_or(0);
        entry.1 += row.try_get::<i64,_>("scam_detections").unwrap_or(0);
        entry.2 += row.try_get::<f64,_>("confidence_sum").unwrap_or(0.0);
        entry.3 += row.try_get::<i64,_>("confidence_count").unwrap_or(0);
        entry.4 += row.try_get::<i64,_>("deleted_messages").unwrap_or(0);
        entry.5 += row.try_get::<i64,_>("timeouts").unwrap_or(0);
    }
    let mut buckets = Vec::with_capacity(periods as usize);
    let mut total = (0_i64,0_i64,0.0_f64,0_i64,0_i64,0_i64);
    for index in 0..periods {
        let timestamp = start + index * bucket_seconds;
        let item = values.get(&timestamp).copied().unwrap_or((0,0,0.0,0,0,0));
        total.0 += item.0; total.1 += item.1; total.2 += item.2; total.3 += item.3; total.4 += item.4; total.5 += item.5;
        buckets.push(AnalyticsBucket {
            timestamp, messages: item.0, scam_detections: item.1,
            average_confidence: (item.3 > 0).then_some(item.2 / item.3 as f64),
            deleted_messages: item.4, timeouts: item.5,
        });
    }
    Ok(Json(AnalyticsResponse {
        guild_id, range: range.to_owned(), timezone: "UTC",
        totals: AnalyticsBucket {
            timestamp: current, messages: total.0, scam_detections: total.1,
            average_confidence: (total.3 > 0).then_some(total.2 / total.3 as f64),
            deleted_messages: total.4, timeouts: total.5,
        },
        buckets,
    }))
}

pub fn router(db: SqlitePool, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String, bot_token: String, logging_identifier: String, timeout_days: u64, runtime_info: SharedRuntimeInfo) -> Router {
    let state = AppState { db, oauth_client_id, oauth_client_secret, oauth_redirect_uri, bot_token, logging_identifier, timeout_days, http: reqwest::Client::new(), runtime_info };
    Router::new()
        .route("/health", get(health))
        .route("/api/health", get(health))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(oauth_callback))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/runtime", get(get_runtime_info))
        .route("/api/guilds/:guild_id/beta-status", get(beta_status))
        .route("/api/guilds/:guild_id/moderation", get(moderation))
        .route("/api/guilds/:guild_id/analytics", get(analytics))
        .route("/api/guilds/:guild_id/protection-options", get(protection_options))
        .route("/api/guilds/:guild_id/settings", get(get_settings).put(save_settings))
        .nest_service("/", ServeDir::new("dashboard"))
        .with_state(state)
}

pub async fn serve(bind: &str, db: SqlitePool, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String, bot_token: String, logging_identifier: String, timeout_days: u64, runtime_info: SharedRuntimeInfo) -> anyhow::Result<()> {
    let address: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Aegis dashboard server listening");
    axum::serve(listener, router(db, oauth_client_id, oauth_client_secret, oauth_redirect_uri, bot_token, logging_identifier, timeout_days, runtime_info)).await?;
    Ok(())
}
