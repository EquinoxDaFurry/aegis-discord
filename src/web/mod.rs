mod db;
mod routes;

use anyhow::Result;
use std::path::PathBuf;

pub async fn run(bind: String, database_path: PathBuf) -> Result<()> {
    let pool = db::connect(&database_path).await?;
    tracing::info!(path = %database_path.display(), "Dashboard database ready");
    routes::serve(&bind, pool).await
}
