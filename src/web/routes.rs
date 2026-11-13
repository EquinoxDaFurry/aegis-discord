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
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{net::SocketAddr, time::{SystemTime, UNIX_EPOCH}};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    db: SqlitePool,
    oauth_client_id: String,
    oauth_client_secret: String,
    oauth_redirect_uri: String,
    http: reqwest::Client,
    runtime_info: SharedRuntimeInfo,
}

#[derive(Serialize)]
struct HealthResponse { status: &'static str, database: &'static str }

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

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct ServerSettings {
    enabled: bool,
    timeout_days: u8,
    delete_threshold: f64,
    logging_channel_id: Option<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self { enabled: true, timeout_days: 7, delete_threshold: 0.805, logging_channel_id: None }
    }
}

#[derive(Serialize)]
struct SettingsResponse {
    guild_id: String,
    settings: ServerSettings,
    updated_at: Option<i64>,
}

#[derive(Deserialize)]
struct SettingsInput {
    enabled: bool,
    timeout_days: u8,
    delete_threshold: f64,
    logging_channel_id: Option<String>,
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

async fn runtime_info(State(state): State<AppState>) -> Json<Option<super::RuntimeInfo>> {
    Json(state.runtime_info.read().await.clone())
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
    if !(1..=30).contains(&input.timeout_days) || !input.delete_threshold.is_finite()
        || !(0.5..=1.0).contains(&input.delete_threshold) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let logging_channel_id = input.logging_channel_id.filter(|v| !v.trim().is_empty());
    if logging_channel_id.as_ref().is_some_and(|v| !v.chars().all(|c| c.is_ascii_digit()) || v.len() > 25) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let settings = ServerSettings { enabled: input.enabled, timeout_days: input.timeout_days,
        delete_threshold: input.delete_threshold, logging_channel_id };
    let json = serde_json::to_string(&settings).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let updated = now();
    sqlx::query("INSERT INTO guild_configs(guild_id, config_json, updated_at) VALUES(?, ?, ?) ON CONFLICT(guild_id) DO UPDATE SET config_json=excluded.config_json, updated_at=excluded.updated_at")
        .bind(&guild.id).bind(json).bind(updated).execute(&state.db).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SettingsResponse { guild_id, settings, updated_at: Some(updated) }))
}

pub fn router(db: SqlitePool, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String, runtime_info: SharedRuntimeInfo) -> Router {
    let state = AppState { db, oauth_client_id, oauth_client_secret, oauth_redirect_uri, http: reqwest::Client::new(), runtime_info };
    Router::new()
        .route("/health", get(health))
        .route("/api/health", get(health))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(oauth_callback))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/runtime", get(runtime_info))
        .route("/api/guilds/:guild_id/settings", get(get_settings).put(save_settings))
        .nest_service("/", ServeDir::new("dashboard"))
        .with_state(state)
}

pub async fn serve(bind: &str, db: SqlitePool, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String, runtime_info: SharedRuntimeInfo) -> anyhow::Result<()> {
    let address: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Aegis dashboard server listening");
    axum::serve(listener, router(db, oauth_client_id, oauth_client_secret, oauth_redirect_uri, runtime_info)).await?;
    Ok(())
}
