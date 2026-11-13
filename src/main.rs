mod config;
mod detection;
mod discord;
mod web;
use anyhow::{Context as _, Result};
use config::Config;
use detection::{image::ImageDetector, text::TextDetector, RuleDatabase};
use serenity::{all::GatewayIntents, Client};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    if dotenvy::dotenv().is_err() { let _ = dotenvy::from_path("../.env"); }
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "aegis=info,serenity=warn".into())
    ).init();
    let config = Config::from_env()?;
    let rules = RuleDatabase::load(&config.database_path)
        .with_context(|| format!("loading Aegis database at {}", config.database_path.display()))?;
    info!("Loaded {} text rules and {} image hashes", rules.text_rule_count(), rules.image_hash_count());
    let text_detector = Arc::new(TextDetector::from_rules(&rules)?);
    let image_detector = Arc::new(ImageDetector::from_rules(&rules, config.image_cache_entries, config.delete_threshold));
    let handler = discord::Handler::new(config.clone(), text_detector, image_detector);
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILD_MEMBERS;
    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(handler).await.context("creating Discord client")?;
    info!("Starting Aegis 2.0 (Rust)");
    let dashboard_bind = config.dashboard_bind.clone();
    let dashboard_database = config.dashboard_database_path.clone();\n    let oauth_client_id = config.dashboard_oauth_client_id.clone();\n    let oauth_client_secret = config.dashboard_oauth_client_secret.clone();\n    let oauth_redirect_uri = config.dashboard_oauth_redirect_uri.clone();
    let mut dashboard_task = tokio::spawn(async move {
        web::run(dashboard_bind, dashboard_database, oauth_client_id, oauth_client_secret, oauth_redirect_uri).await
    });
    tokio::select! {
        result = client.start() => {
            dashboard_task.abort();
            result.context("Discord client stopped")?;
        }
        result = &mut dashboard_task => {
            match result {
                Ok(Ok(())) => anyhow::bail!("Dashboard server exited unexpectedly"),
                Ok(Err(error)) => return Err(error),
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}
