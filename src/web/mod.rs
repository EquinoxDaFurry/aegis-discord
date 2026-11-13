mod db;
mod routes;

use anyhow::Result;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone, Serialize)]
pub struct RuntimeInfo {
    pub bot_user_id: String,
    pub bot_username: String,
    pub is_beta: bool,
}

pub type SharedRuntimeInfo = Arc<RwLock<Option<RuntimeInfo>>>;

pub async fn run(bind: String, database_path: PathBuf, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String, bot_token: String, logging_identifier: String, timeout_days: u64, runtime_info: SharedRuntimeInfo) -> Result<()> {
    let pool = db::connect(&database_path).await?;
    tracing::info!(path = %database_path.display(), "Dashboard database ready");
    routes::serve(&bind, pool, oauth_client_id, oauth_client_secret, oauth_redirect_uri, bot_token, logging_identifier, timeout_days, runtime_info).await
}

pub async fn connect_dashboard_database(path: &std::path::Path) -> Result<sqlx::SqlitePool> {
    db::connect(path).await
}
