use anyhow::{Context, Result};
use std::{env, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub discord_token: String,
    pub database_path: PathBuf,
    pub delete_threshold: f64,
    pub max_image_bytes: usize,
    pub image_cache_entries: usize,
    pub logging_identifier: String,
    pub timeout_days: u64,
    pub cooldown_seconds: u64,
    pub dm_report_server: String,
    pub false_positive_channel_id: Option<String>,
    pub website_api: Option<String>,
    pub api_key: Option<String>,
    pub dashboard_bind: String,
    pub dashboard_database_path: PathBuf,\n    pub dashboard_oauth_client_id: String,\n    pub dashboard_oauth_client_secret: String,\n    pub dashboard_oauth_redirect_uri: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let discord_token = env::var("BOT").or_else(|_| env::var("DISCORD_TOKEN"))
            .context("set BOT or DISCORD_TOKEN in the environment")?;
        Ok(Self {
            discord_token,
            database_path: env::var_os("AEGIS_DATABASE").map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("database.aegis")),
            delete_threshold: env::var("AEGIS_DELETE_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.805),
            max_image_bytes: env::var("AEGIS_MAX_IMAGE_BYTES").ok().and_then(|v| v.parse().ok()).unwrap_or(3 * 1024 * 1024),
            image_cache_entries: env::var("AEGIS_IMAGE_CACHE_ENTRIES").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000),
            logging_identifier: env::var("AEGIS_LOGGING_IDENTIFIER").unwrap_or_else(|_| "aegis-logs".to_owned()),
            timeout_days: env::var("AEGIS_TIMEOUT_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(7),
            cooldown_seconds: env::var("AEGIS_COOLDOWN_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(900),
            dm_report_server: env::var("AEGIS_DM_REPORT_SERVER").unwrap_or_else(|_| "Not configured".to_owned()),
            false_positive_channel_id: env::var("AEGIS_FALSE_POSITIVE_CHANNEL_ID").ok()
                .filter(|value| !value.trim().is_empty()),
            website_api: env::var("WEBSITE_API").ok().filter(|value| !value.trim().is_empty()),
            api_key: env::var("API_KEY").ok().filter(|value| !value.trim().is_empty()),
            dashboard_bind: env::var("AEGIS_DASHBOARD_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_owned()),
            dashboard_database_path: env::var_os("AEGIS_DASHBOARD_DATABASE").map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("aegis-dashboard.sqlite3")),
            dashboard_oauth_client_id: env::var("AEGIS_DISCORD_CLIENT_ID").unwrap_or_default(),
            dashboard_oauth_client_secret: env::var("AEGIS_DISCORD_CLIENT_SECRET").unwrap_or_default(),
            dashboard_oauth_redirect_uri: env::var("AEGIS_DISCORD_REDIRECT_URI").unwrap_or_default(),
        })
    }
}
