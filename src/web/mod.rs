mod db;
mod routes;

use anyhow::Result;
use std::path::PathBuf;

pub async fn run(bind: String, database_path: PathBuf, oauth_client_id: String, oauth_client_secret: String, oauth_redirect_uri: String) -> Result<()> {
    let pool = db::connect(&database_path).await?;
    tracing::info!(path = %database_path.display(), "Dashboard database ready");
    routes::serve(&bind, pool, oauth_client_id, oauth_client_secret, oauth_redirect_uri).await
}
