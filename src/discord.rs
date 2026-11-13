use crate::{config::Config, detection::{image::ImageDetector, text::TextDetector, DetectionKind}};
use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serenity::gateway::ActivityData;
use serenity::{all::{ActionRowComponent, ButtonStyle, Context, CreateActionRow, CreateAttachment, CreateButton, CreateCommand, CreateEmbed, CreateInputText, CreateInteractionResponse, CreateInvite, CreateMessage, CreateModal, EditMessage, EventHandler, InputTextStyle, Interaction, Message, Timestamp, UserId}, prelude::Mentionable};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{Duration, SystemTime, UNIX_EPOCH}};
use tracing::{info, warn};

const BETA_APPLICATION_ID: u64 = 1532972276805537792;

#[derive(Clone)]
struct FalsePositiveData {
    user_mention: String,
    user_id: u64,
    guild_name: String,
    guild_id: u64,
    channel_name: String,
    reason: String,
    flagged_content: String,
    message_url: String,
    images: Vec<(String, Vec<u8>)>,
}

pub struct Handler {
    config: Config,
    runtime_info: crate::web::SharedRuntimeInfo,
    text_detector: Arc<TextDetector>,
    image_detector: Arc<ImageDetector>,
    http: HttpClient,
    incident_cooldowns: Mutex<HashMap<(u64, u64), u64>>,
    false_positive_reports: Mutex<HashMap<u64, FalsePositiveData>>,
    false_positive_submitted: Mutex<HashSet<u64>>,
    heartbeat_started: AtomicBool,
    messages_since_heartbeat: Arc<AtomicU64>,
    threats_since_heartbeat: Arc<AtomicU64>,
    image_semaphore: Arc<tokio::sync::Semaphore>,
    analytics_db: sqlx::SqlitePool,
}
impl Handler {
    pub fn new(config: Config, text_detector: Arc<TextDetector>, image_detector: Arc<ImageDetector>, runtime_info: crate::web::SharedRuntimeInfo, analytics_db: sqlx::SqlitePool) -> Self {
        Self {
            config, runtime_info, text_detector, image_detector, analytics_db,
            http: HttpClient::builder().timeout(std::time::Duration::from_secs(15))
                .build().expect("reqwest client configuration is valid"),
            incident_cooldowns: Mutex::new(HashMap::new()),
            false_positive_reports: Mutex::new(HashMap::new()),
            false_positive_submitted: Mutex::new(HashSet::new()),
            heartbeat_started: AtomicBool::new(false),
            messages_since_heartbeat: Arc::new(AtomicU64::new(0)),
            threats_since_heartbeat: Arc::new(AtomicU64::new(0)),
            image_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }

    async fn record_analytics(&self, guild_id: u64, messages: i64, detections: i64, confidence: Option<f64>, deleted: i64, timeouts: i64) {
        let hour_start = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64).div_euclid(3600) * 3600;
        let result = sqlx::query("INSERT INTO analytics_hourly(guild_id,hour_start,messages,scam_detections,confidence_sum,confidence_count,deleted_messages,timeouts) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(guild_id,hour_start) DO UPDATE SET messages=messages+excluded.messages, scam_detections=scam_detections+excluded.scam_detections, confidence_sum=confidence_sum+excluded.confidence_sum, confidence_count=confidence_count+excluded.confidence_count, deleted_messages=deleted_messages+excluded.deleted_messages, timeouts=timeouts+excluded.timeouts")
            .bind(guild_id.to_string()).bind(hour_start).bind(messages).bind(detections)
            .bind(confidence.unwrap_or(0.0)).bind(if confidence.is_some() { 1_i64 } else { 0_i64 })
            .bind(deleted).bind(timeouts).execute(&self.analytics_db).await;
        if let Err(error) = result {
            warn!(%error, guild_id, "Could not update analytics aggregates");
        }
    }

    async fn guild_settings(&self, guild_id: u64) -> serde_json::Value {
        let stored = sqlx::query_as::<_, (String,)>("SELECT config_json FROM guild_configs WHERE guild_id = ?")
            .bind(guild_id.to_string()).fetch_optional(&self.analytics_db).await;
        match stored {
            Ok(Some((json,))) => serde_json::from_str(&json).unwrap_or_else(|error| {
                warn!(%error, guild_id, "Invalid per-server settings JSON; using defaults");
                serde_json::json!({})
            }),
            Ok(None) => serde_json::json!({}),
            Err(error) => {
                warn!(%error, guild_id, "Could not read per-server settings; using defaults");
                serde_json::json!({})
            }
        }
    }

    fn setting_bool(settings: &serde_json::Value, key: &str, default: bool) -> bool {
        settings.get(key).and_then(serde_json::Value::as_bool).unwrap_or(default)
    }

    fn setting_string_list_contains(settings: &serde_json::Value, key: &str, value: &str) -> bool {
        settings.get(key).and_then(serde_json::Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(value)))
    }

    fn template_section(content: &str, label: &str) -> Option<String> {
        let labels = ["User", "Reason", "Flagged Content", "Channel", "Deleted", "Timeout"];
        let mut found = false;
        let mut values = Vec::new();
        for line in content.lines() {
            if !found {
                if line.trim() == label { found = true; }
                continue;
            }
            if labels.iter().any(|next| *next != label && line.trim() == *next)
                || line.trim_start().starts_with("[Button:") {
                break;
            }
            values.push(line);
        }
        let value = values.join("\n").trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    fn render_template(
        settings: &serde_json::Value,
        key: &str,
        default: &str,
        values: &[(&str, String)],
        limit: usize,
    ) -> Option<String> {
        let template = settings.get("message_templates").and_then(|items| items.get(key));
        if template.and_then(|item| item.get("enabled")).and_then(serde_json::Value::as_bool) == Some(false) {
            return None;
        }
        let mut content = template.and_then(|item| item.get("content"))
            .and_then(serde_json::Value::as_str).unwrap_or(default).to_owned();
        for (name, value) in values {
            content = content.replace(&format!("{{{name}}}"), value);
        }
        Some(content.chars().take(limit).collect())
    }

    async fn send_error_dm(&self, ctx: &Context, error_text: &str) {
        if let Some(owner_id) = std::env::var("AEGIS_OWNER_ID").ok().and_then(|value| value.parse::<u64>().ok()).map(UserId::new) {
        if let Ok(user) = ctx.http.get_user(owner_id).await {
            let content = format!("⚠️ **Bot Error**\n\n```text\n{}
```", error_text.chars().take(1900).collect::<String>());
            let _ = user.dm(&ctx.http, CreateMessage::new().content(content)).await;
        }
        }
    }

    fn on_cooldown(&self, guild_id: u64, user_id: u64) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let Ok(mut cooldowns) = self.incident_cooldowns.lock() else { return false };
        if cooldowns.get(&(guild_id, user_id)).is_some_and(|last| now.saturating_sub(*last) < self.config.cooldown_seconds) {
            return true;
        }
        cooldowns.insert((guild_id, user_id), now);
        cooldowns.retain(|_, last| now.saturating_sub(*last) < self.config.cooldown_seconds.saturating_mul(4));
        false
    }

    async fn scan_attachments(&self, ctx: &Context, message: &Message, campaign_flagged: bool, threshold: f64) -> Option<(String, Vec<(String, Vec<u8>)>, f64)> {
        let mut best: Option<(f64, String)> = None;
        let mut flagged = Vec::new();
        for attachment in &message.attachments {
            let is_image = attachment.content_type.as_deref().is_some_and(|kind| kind.starts_with("image/"));
            if !is_image || attachment.size as usize > self.config.max_image_bytes { continue; }
            let _permit = match self.image_semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(error) => { warn!(%error, "Image scan semaphore closed"); continue; }
            };
            let response = match self.http.get(&attachment.url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => { warn!(status = %r.status(), "Image download returned non-success status"); continue; }
                Err(error) => {
                    warn!(%error, "Unable to download image attachment");
                    self.send_error_dm(ctx, &format!("Image download failed: {error}")).await;
                    continue;
                }
            };
            if response.content_length().is_some_and(|length| length > self.config.max_image_bytes as u64) { continue; }
            let Ok(bytes) = response.bytes().await else { continue };
            if bytes.len() > self.config.max_image_bytes { continue; }
            let detector = self.image_detector.clone();
            let scan_bytes = bytes.to_vec();
            let scan_result = tokio::time::timeout(Duration::from_secs(30),
                tokio::task::spawn_blocking(move || detector.scan_with_threshold(&scan_bytes, threshold))).await;
            let scan_result = match scan_result {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => { warn!(%error, "Image scan worker failed"); continue; }
                Err(_) => { warn!(attachment = %attachment.filename, "Image scan timed out"); continue; }
            };
            match scan_result {
                Ok(Some(scan)) => {
                    if std::env::var("AEGIS_OWNER_ID").ok().and_then(|value| value.parse::<u64>().ok()) == Some(message.author.id.get()) {
                        if let Some(candidate) = scan.matches.first() {
                            info!(sha256 = %scan.sha256, filename = %candidate.filename,
                                campaign = %candidate.campaign, confidence = candidate.confidence,
                                "Debug image scan best match");
                        }
                    }
                    let campaign_bonus = if campaign_flagged { 0.03 } else { 0.0 };
                    let adjusted_confidence = scan.confidence + campaign_bonus;
                    if best.as_ref().is_none_or(|(confidence, _)| adjusted_confidence > *confidence) {
                        let mut details = format!("Image attachment matched a known scam campaign.\nConfidence: {:.2}%\nOriginal Confidence: {:.2}%",
                            adjusted_confidence * 100.0, scan.confidence * 100.0);
                        if campaign_bonus > 0.0 { details.push_str("\nCampaign Bonus: +3.00%"); }
                        best = Some((adjusted_confidence, details));
                    }
                    flagged.push((attachment.filename.clone(), bytes.to_vec()));
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, "Image scan failed");
                    self.send_error_dm(ctx, &format!("Image scan failed: {error}")).await;
                }
            }
        }
        best.map(|(confidence, reason)| (reason, flagged, confidence))
    }

    async fn send_audit_log(&self, ctx: &Context, message: &Message, guild_name: &str, reason: &str, flagged_content: &str, deleted: bool, timeout_status: &str, timeout_until: Option<i64>, images: Vec<(String, Vec<u8>)>, settings: &serde_json::Value) -> Option<u64> {
        let Some(guild_id) = message.guild_id else { return None };
        let channels = match guild_id.channels(&ctx.http).await {
            Ok(channels) => channels,
            Err(error) => { warn!(%error, "Could not inspect server channels for Aegis logging channel"); return; }
        };
        let configured = settings.get("logging_channel_id").and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<u64>().ok())
            .map(serenity::all::ChannelId::new);
        let target = configured.and_then(|id| channels.get(&id))
            .or_else(|| channels.values().find(|channel| channel.topic.as_deref()
                .is_some_and(|topic| topic.contains(&self.config.logging_identifier))));
        let Some(channel) = target else {
            warn!(guild_id = %guild_id, "No configured or identifier-matched Aegis logging channel was found");
            return;
        };
        let safe_reason = flagged_content.chars().take(1000).collect::<String>().replace("```", "'''");
        let user_mention = message.author.mention().to_string();
        let channel_mention = message.channel_id.mention().to_string();
        let detection_text = Self::render_template(settings, "detection",
            "🚨 Scam Detected\nPossible scam content detected.\n\nUser\n{user}\n`{user_id}`\n\nReason\n{reason}\n\nFlagged Content\n```text\n{flagged_content}\n```\n\nChannel\n{channel}\n\nDeleted\n{deleted}\n\nTimeout\n{timeout_status}",
            &[("user", user_mention.clone()), ("user_id", message.author.id.to_string()),
              ("reason", reason.to_owned()), ("flagged_content", safe_reason.clone()),
              ("channel", channel_mention.clone()), ("deleted", deleted.to_string()),
              ("timeout_status", timeout_status.to_owned())], 3900)
            .unwrap_or_else(|| "🚨 Scam Detected\nPossible scam content detected.".to_owned());
        let has_sections = Self::template_section(&detection_text, "User").is_some()
            && Self::template_section(&detection_text, "Reason").is_some();
        let configured_title = detection_text.lines().next().unwrap_or("🚨 Scam Detected").trim();
        let embed_title = if configured_title.is_empty() { "🚨 Scam Detected".to_owned() }
            else if configured_title.contains("Scam Detected") { configured_title.to_owned() }
            else { format!("{} · Scam Detected", configured_title) };
        let description = if has_sections {
            detection_text.lines().skip(1).take_while(|line| line.trim() != "User").collect::<Vec<_>>().join("\n").trim().to_owned()
        } else if detection_text.lines().count() > 1 {
            detection_text.lines().skip(1).collect::<Vec<_>>().join("\n")
        } else { detection_text.clone() };
        let user_field = Self::template_section(&detection_text, "User")
            .unwrap_or_else(|| format!("{}\n`{}`", user_mention, message.author.id));
        let reason_field = Self::template_section(&detection_text, "Reason").unwrap_or_else(|| reason.to_owned());
        let flagged_field = Self::template_section(&detection_text, "Flagged Content")
            .unwrap_or_else(|| format!("```text\n{}\n```", safe_reason));
        let channel_field = Self::template_section(&detection_text, "Channel").unwrap_or_else(|| channel_mention.clone());
        let deleted_field = Self::template_section(&detection_text, "Deleted").unwrap_or_else(|| deleted.to_string());
        let timeout_field = Self::template_section(&detection_text, "Timeout").unwrap_or_else(|| timeout_status.to_owned());
        let embed = CreateEmbed::new()
            .title(embed_title)
            .description(description)
            .timestamp(Timestamp::now())
            .color(0xED4245)
            .field("User", user_field, false)
            .field("Reason", reason_field, false)
            .field("Flagged Content", flagged_field, false)
            .field("Channel", channel_field, true)
            .field("Deleted", deleted_field, true)
            .field("Timeout", timeout_field, false);
        let button = CreateButton::new("aegis_false_positive")
            .label("False Positive").emoji('⚠').style(ButtonStyle::Secondary);
        let mut audit = CreateMessage::new().embed(embed)
            .components(vec![CreateActionRow::Buttons(vec![button])]);
        for (filename, bytes) in &images {
            audit = audit.add_file(CreateAttachment::bytes(bytes.clone(), filename.clone()));
        }
        match channel.send_message(&ctx.http, audit).await {
            Ok(log_message) => {
                let created_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                let case_status = if timeout_until.is_some() { "active" } else { "failed" };
                if let Err(error) = sqlx::query("INSERT OR IGNORE INTO moderation_cases(case_id, guild_id, guild_name, member_id, moderator_id, reason, source, timeout_until, status, created_at, original_message_url) VALUES(?, ?, ?, ?, NULL, ?, 'automatic_detection', ?, ?, ?, ?)")
                    .bind(log_message.id.to_string()).bind(guild_id.to_string()).bind(guild_name.chars().take(100).collect::<String>())
                    .bind(message.author.id.to_string()).bind(reason.chars().take(1000).collect::<String>()).bind(timeout_until).bind(case_status)
                    .bind(created_at).bind(message.link()).execute(&self.analytics_db).await {
                    warn!(%error, case_id = %log_message.id, "Could not persist moderation case for owner console");
                }
                if let Ok(mut reports) = self.false_positive_reports.lock() {
                    reports.insert(log_message.id.get(), FalsePositiveData {
                        user_mention,
                        user_id: message.author.id.get(),
                        guild_name: message.guild_id.and_then(|id| ctx.cache.guild(id).map(|g| g.name.clone())).unwrap_or_else(|| "Unknown server".to_owned()),
                        guild_id: guild_id.get(),
                        channel_name: channel_mention,
                        reason: reason.chars().take(1000).collect(),
                        flagged_content: safe_reason,
                        message_url: message.link(),
                        images: images.clone(),
                    });
                }
                Some(log_message.id.get())
            }
            Err(error) => {
                warn!(%error, "Could not send Aegis audit log");
                self.send_error_dm(ctx, &format!("Failed to send scam audit log: {error}")).await;
                None
            }
        }
    }

    async fn propagate_cross_server_detection(
        &self, ctx: &Context, source_message: &Message, source_reason: &str,
        flagged_content: &str, confidence: Option<f64>, source_settings: &serde_json::Value,
    ) {
        let Some(source_guild_id) = source_message.guild_id else { return; };
        if !Self::setting_bool(source_settings, "cross_server_intelligence", true)
            || Self::setting_bool(source_settings, "dry_run", false) { return; }
        let user_id = source_message.author.id;
        let source_guild_name = source_guild_id.to_partial_guild(&ctx.http).await
            .map(|guild| guild.name).unwrap_or_else(|_| "Unknown server".to_owned());
        let confidence_text = confidence.map(|value| format!("{:.2}%", value.clamp(0.0, 1.0) * 100.0))
            .unwrap_or_else(|| "Not available (text rule match)".to_owned());
        for target_guild_id in ctx.cache.guilds() {
            if target_guild_id == source_guild_id { continue; }
            let target_settings = self.guild_settings(target_guild_id.get()).await;
            if !Self::setting_bool(&target_settings, "enabled", true)
                || !Self::setting_bool(&target_settings, "cross_server_intelligence", true)
                || !Self::setting_bool(&target_settings, "auto_timeout", true)
                || Self::setting_bool(&target_settings, "dry_run", false) { continue; }
            let reserved = sqlx::query("INSERT OR IGNORE INTO cross_server_actions(source_guild_id, target_guild_id, member_id, source_message_id, created_at) VALUES(?, ?, ?, ?, ?)")
                .bind(source_guild_id.get().to_string()).bind(target_guild_id.get().to_string())
                .bind(user_id.get().to_string()).bind(source_message.id.get().to_string())
                .bind(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
                .execute(&self.analytics_db).await;
            match reserved {
                Ok(result) if result.rows_affected() == 0 => continue,
                Ok(_) => {},
                Err(error) => { warn!(%error, target_guild_id = %target_guild_id, "Could not reserve cross-server action"); continue; }
            }
            let mut member = match target_guild_id.member(&ctx.http, user_id).await {
                Ok(member) => member,
                Err(_) => continue,
            };
            // CacheRef is deliberately scoped here: Serenity cache references are not Send
            // and must never be held across an await in an event-handler future.
            let (is_admin, cached_target_name) = {
                let target_guild = ctx.cache.guild(target_guild_id);
                target_guild.as_ref().map(|guild| (
                    guild.member_permissions(&member).administrator(),
                    guild.name.clone(),
                )).unwrap_or((false, String::new()))
            };
            if Self::setting_bool(&target_settings, "exempt_admins", true) && is_admin { continue; }
            if target_settings.get("exempt_role_ids").and_then(serde_json::Value::as_array).is_some_and(|roles| {
                member.roles.iter().any(|role| {
                    let id = role.get().to_string();
                    roles.iter().any(|item| item.as_str() == Some(id.as_str()))
                })
            }) { continue; }
            let days = target_settings.get("timeout_days").and_then(serde_json::Value::as_u64)
                .unwrap_or(self.config.timeout_days).clamp(1, 7);
            let until = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
                .saturating_add(days.saturating_mul(86_400));
            let (timeout_status, timeout_until) = match Timestamp::from_unix_timestamp(until as i64) {
                Ok(timestamp) => match member.disable_communication_until_datetime(&ctx.http, timestamp).await {
                    Ok(_) => ("Timed out successfully".to_owned(), Some(until as i64)),
                    Err(error) => {
                        warn!(%error, target_guild_id = %target_guild_id, user_id = %user_id, "Cross-server timeout failed");
                        ("Unable to timeout user (permissions or role hierarchy)".to_owned(), None)
                    }
                },
                Err(error) => ("Unable to calculate timeout expiry".to_owned(), {
                    warn!(%error, "Could not calculate cross-server timeout expiry"); None
                }),
            };
            if timeout_until.is_some() { self.record_analytics(target_guild_id.get(), 0, 0, None, 0, 1).await; }
            let channels = match target_guild_id.channels(&ctx.http).await {
                Ok(channels) => channels,
                Err(error) => { warn!(%error, target_guild_id = %target_guild_id, "Could not list destination logging channels"); continue; }
            };
            let configured = target_settings.get("logging_channel_id").and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse::<u64>().ok()).map(serenity::all::ChannelId::new);
            let channel = configured.and_then(|id| channels.get(&id))
                .or_else(|| channels.values().find(|channel| channel.topic.as_deref()
                    .is_some_and(|topic| topic.contains(&self.config.logging_identifier))));
            let Some(channel) = channel else {
                warn!(target_guild_id = %target_guild_id, "No logging channel found for cross-server action"); continue;
            };
            let target_name = if cached_target_name.is_empty() {
                target_guild_id.to_partial_guild(&ctx.http).await
                    .map(|guild| guild.name).unwrap_or_else(|_| "Unknown server".to_owned())
            } else {
                cached_target_name
            };
            let safe_flagged = flagged_content.chars().take(1200).collect::<String>().replace("```", "'''");
            let embed = CreateEmbed::new().title("Cross Server Intelligence Network")
                .description(format!("Aegis detected a threat in {} and applied a network action in {}.", source_guild_name, target_name))
                .timestamp(Timestamp::now()).color(0xED4245)
                .field("User", format!("{}\n`{}`", user_id.mention(), user_id), false)
                .field("Reason", "Cross Server Intelligence Network", false)
                .field("Source Detection", source_reason.chars().take(500).collect::<String>(), true)
                .field("Confidence", confidence_text.clone(), true)
                .field("Flagged Content", safe_flagged.chars().take(1000).collect::<String>(), false)
                .field("Source Server", format!("{}\n`{}`", source_guild_name, source_guild_id), false)
                .field("Original Message", source_message.link(), false)
                .field("Timeout", format!("{}{}", timeout_status, timeout_until.map(|_| format!(" ({} days)", days)).unwrap_or_default()), false);
            let button = CreateButton::new("aegis_false_positive").label("False Positive").emoji('⚠').style(ButtonStyle::Secondary);
            match channel.send_message(&ctx.http, CreateMessage::new().embed(embed)
                .components(vec![CreateActionRow::Buttons(vec![button])])).await {
                Ok(log_message) => {
                    let created_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                    let status = if timeout_until.is_some() { "active" } else { "failed" };
                    if let Err(error) = sqlx::query("INSERT OR IGNORE INTO moderation_cases(case_id, guild_id, guild_name, member_id, moderator_id, reason, source, timeout_until, status, created_at, original_message_url) VALUES(?, ?, ?, ?, NULL, ?, 'cross_server_intelligence', ?, ?, ?, ?)")
                        .bind(log_message.id.get().to_string()).bind(target_guild_id.get().to_string())
                        .bind(target_name.chars().take(100).collect::<String>()).bind(user_id.get().to_string())
                        .bind("Cross Server Intelligence Network").bind(timeout_until).bind(status)
                        .bind(created_at).bind(source_message.link()).execute(&self.analytics_db).await {
                        warn!(%error, "Could not persist cross-server moderation case");
                    }
                    if let Err(error) = sqlx::query("UPDATE cross_server_actions SET case_id = ? WHERE source_guild_id = ? AND target_guild_id = ? AND member_id = ? AND source_message_id = ?")
                        .bind(log_message.id.get().to_string())
                        .bind(source_guild_id.get().to_string())
                        .bind(target_guild_id.get().to_string())
                        .bind(user_id.get().to_string())
                        .bind(source_message.id.get().to_string())
                        .execute(&self.analytics_db).await {
                        warn!(%error, case_id = %log_message.id, "Could not link cross-server action to owner case");
                    }
                    if let Ok(mut reports) = self.false_positive_reports.lock() {
                        reports.insert(log_message.id.get(), FalsePositiveData {
                            user_mention: user_id.mention().to_string(), user_id: user_id.get(),
                            guild_name: target_name, guild_id: target_guild_id.get(),
                            channel_name: channel.id.mention().to_string(),
                            reason: "Cross Server Intelligence Network".to_owned(),
                            flagged_content: safe_flagged, message_url: source_message.link(), images: Vec::new(),
                        });
                    }
                }
                Err(error) => warn!(%error, target_guild_id = %target_guild_id, "Could not send cross-server audit log"),
            }
        }
    }

    async fn handle_detection(&self, ctx: &Context, message: &Message, reason: &str, flagged_content: &str, images: Vec<(String, Vec<u8>)>, confidence: Option<f64>, settings: &serde_json::Value) {
        self.threats_since_heartbeat.fetch_add(1, Ordering::Relaxed);
        let Some(guild_id) = message.guild_id else { return };
        let dry_run = Self::setting_bool(settings, "dry_run", false);
        let auto_delete = Self::setting_bool(settings, "auto_delete", true) && !dry_run;
        let auto_timeout = Self::setting_bool(settings, "auto_timeout", true) && !dry_run;
        let timeout_days = settings.get("timeout_days").and_then(serde_json::Value::as_u64)
            .unwrap_or(self.config.timeout_days).clamp(1, 7);

        let deleted = if auto_delete {
            match message.delete(&ctx.http).await {
                Ok(()) => true,
                Err(error) => {
                    warn!(%error, "Could not delete detected message (check Manage Messages permission)");
                    self.send_error_dm(ctx, &format!("Could not delete detected message: {error}")).await;
                    false
                }
            }
        } else { false };
        self.record_analytics(guild_id.get(), 0, 1, confidence.map(|score| score.clamp(0.0, 1.0)), if deleted { 1 } else { 0 }, 0).await;

        let cooldown = self.on_cooldown(guild_id.get(), message.author.id.get());
        let timeout_status = if dry_run {
            "Dry-run mode: no timeout applied".to_owned()
        } else if !auto_timeout {
            "Automatic timeout disabled".to_owned()
        } else if cooldown {
            "Cooldown active: timeout skipped".to_owned()
        } else {
            match guild_id.member(&ctx.http, message.author.id).await {
                Ok(mut member) => {
                    let until = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
                        .saturating_add(timeout_days.saturating_mul(86_400));
                    match Timestamp::from_unix_timestamp(until as i64) {
                        Ok(timestamp) => match member.disable_communication_until_datetime(&ctx.http, timestamp).await {
                            Ok(_) => "Timed out successfully".to_owned(),
                            Err(error) => {
                                warn!(%error, "Could not timeout affected user");
                                self.send_error_dm(ctx, &format!("Could not timeout detected user: {error}")).await;
                                "⚠️ Unable to timeout user (missing permissions or role hierarchy)".to_owned()
                            }
                        },
                        Err(error) => {
                            warn!(%error, "Could not calculate timeout expiry");
                            "⚠️ Discord API error while timing out".to_owned()
                        }
                    }
                }
                Err(error) => {
                    warn!(%error, "Could not fetch affected guild member");
                    "⚠️ Unable to timeout user (missing permissions or role hierarchy)".to_owned()
                }
            }
        };

        let timeout_until = if timeout_status == "Timed out successfully" {
            Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
                + timeout_days.saturating_mul(86_400) as i64)
        } else { None };
        let case_id = self.send_audit_log(ctx, message, &guild_name, reason, flagged_content, deleted, &timeout_status, timeout_until, images.clone(), settings).await;
        if timeout_status == "Timed out successfully" {
            self.record_analytics(guild_id.get(), 0, 0, None, 0, 1).await;
        }
        let guild_name = match guild_id.to_partial_guild(&ctx.http).await {
            Ok(guild) => guild.name,
            Err(_) => "Unknown server".to_owned(),
        };
        let flagged_text = flagged_content.chars().take(1500).collect::<String>();
        if timeout_status == "Timed out successfully" {
            let default_dm = "Hello {user_id},\n\n⚠️ **Security Alert**\n\nYour account sent content commonly associated with scam campaigns.\n\nYour account may have been compromised.\n\nPlease:\n\n🔒 Reset your Discord password\n🛡️ Enable Two-Factor Authentication\n🦠 Run malware scans\n🔗 Review Authorized Apps\n\nYou have been timed out in:\n\n**{server}**\n\nfor **{duration} days**.\n\nDetected location:\n\n{channel}\n\nFlagged Content:\n\n```text\n{flagged_content}\n```\n\n-# Anti-Scam Protection System";
            let dm_text = Self::render_template(settings, "timeout", default_dm,
                &[("user", message.author.mention().to_string()), ("user_id", format!("<@{}>", message.author.id)),
                  ("server", guild_name.clone()), ("duration", timeout_days.to_string()),
                  ("channel", format!("<#{}>", message.channel_id)), ("reason", reason.to_owned()),
                  ("flagged_content", flagged_text), ("report_server", self.config.dm_report_server.clone()),
                  ("deleted", deleted.to_string()), ("timeout_status", timeout_status.clone())], 1900);
            if let Some(dm_text) = dm_text {
                let mut dm = CreateMessage::new().content(format!("{}\n\n**If this was wrong, please click the button below.**", dm_text));
                if let Some(case_id) = case_id {
                    dm = dm.components(vec![CreateActionRow::Buttons(vec![CreateButton::new(format!("aegis_fp_dm:{case_id}")).label("Report a false positive").style(ButtonStyle::Primary)])]);
                }
                for (filename, bytes) in &images {
                    dm = dm.add_file(CreateAttachment::bytes(bytes.clone(), filename.clone()));
                }
                if let Err(error) = message.author.dm(&ctx.http, dm).await {
                    warn!(%error, "Could not DM affected user");
                    self.send_error_dm(ctx, &format!("Could not DM affected user: {error}")).await;
                }
            }
        }
        self.propagate_cross_server_detection(ctx, message, reason, flagged_content, confidence, settings).await;
    }
}
#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: serenity::all::Ready) {
        info!(user = %ready.user.name, user_id = %ready.user.id, "Aegis connected");
        let is_beta = ready.application.id.get() == BETA_APPLICATION_ID;
        *self.runtime_info.write().await = Some(crate::web::RuntimeInfo {
            bot_user_id: ready.user.id.to_string(),
            bot_username: ready.user.name.clone(),
            is_beta,
        });
        let activity = if is_beta { "AEGIS BETA" } else { "protecting servers" };
        ctx.shard.set_activity(Some(ActivityData::playing(activity)));
        let help_command = CreateCommand::new("help").description("Show information about the bot.");
        if let Err(error) = ctx.http.create_global_command(&help_command).await {
            warn!(%error, "Could not sync the /help command");
            self.send_error_dm(&ctx, &format!("Could not sync /help: {error}")).await;
        }
        let (Some(endpoint), Some(api_key)) = (self.config.website_api.clone(), self.config.api_key.clone()) else {
            info!("Website heartbeat disabled: WEBSITE_API or API_KEY is not configured");
            return;
        };
        if self.heartbeat_started.swap(true, Ordering::AcqRel) { return; }
        let http = self.http.clone();
        let messages = self.messages_since_heartbeat.clone();
        let threats = self.threats_since_heartbeat.clone();
        tokio::spawn(async move {
            loop {
                let (servers, users) = {
                    let guild_ids = ctx.cache.guilds();
                    let mut total_users = 0_u64;
                    for guild_id in &guild_ids {
                        if let Some(guild) = ctx.cache.guild(*guild_id) {
                            total_users = total_users.saturating_add(guild.member_count);
                        }
                    }
                    (guild_ids.len() as u64, total_users)
                };
                let payload = serde_json::json!({
                    "servers": servers,
                    "users": users,
                    "threats": threats.load(Ordering::Relaxed),
                    "messages": messages.load(Ordering::Relaxed)
                });
                match http.post(&endpoint).bearer_auth(&api_key).json(&payload).send().await {
                    Ok(response) => {
                        if !response.status().is_success() {
                            warn!(status = %response.status(), "Website heartbeat returned an HTTP error");
                        }
                        messages.store(0, Ordering::Relaxed);
                        threats.store(0, Ordering::Relaxed);
                    }
                    Err(error) => {
                        warn!(%error, "Website heartbeat failed");
                        if let Some(owner_id) = std::env::var("AEGIS_OWNER_ID").ok().and_then(|value| value.parse::<u64>().ok()).map(UserId::new) {
                            if let Ok(user) = ctx.http.get_user(owner_id).await {
                            let content = format!("⚠️ **Bot Error**\n\n```text\nWebsite heartbeat failed: {}\n```", error);
                            let _ = user.dm(&ctx.http, CreateMessage::new().content(content)).await;
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) if command.data.name == "help" => {
                let embed = CreateEmbed::new().title("🛡️ AEGIS")
                    .description("AEGIS is an automated scam detection and protection bot.")
                    .color(0x3498DB)
                    .field("Detection", "Scans messages and images for known scam content.", false)
                    .field("Protection", "Detected scam messages can be removed and users timed out.", false);
                let response = serenity::all::CreateInteractionResponseMessage::new()
                    .embed(embed).ephemeral(true);
                if let Err(error) = command.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await {
                    self.send_error_dm(&ctx, &format!("Could not respond to /help: {error}")).await;
                }
            }
            Interaction::Component(component) if component.data.custom_id == "aegis_false_positive" || component.data.custom_id.starts_with("aegis_fp_dm:") => {
                let log_id = if component.data.custom_id.starts_with("aegis_fp_dm:") {
                    component.data.custom_id.trim_start_matches("aegis_fp_dm:").parse::<u64>().unwrap_or(0)
                } else { component.message.id.get() };
                if log_id == 0 { return; }
                let report_owner = self.false_positive_reports.lock().ok()
                    .and_then(|reports| reports.get(&log_id).map(|data| data.user_id));
                if report_owner != Some(component.user.id.get()) {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("❌ Only the member involved in this detection can submit this report from their timeout DM.").ephemeral(true);
                    let _ = component.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                }
                let already = self.false_positive_submitted.lock().map(|s| s.contains(&log_id)).unwrap_or(false);
                if already {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("⚠️ This detection has already been marked as a false positive.").ephemeral(true);
                    let _ = component.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                }
                let is_dm_report = component.data.custom_id.starts_with("aegis_fp_dm:");
                let modal = CreateModal::new(format!("aegis_fp_modal:{log_id}:{}", if is_dm_report { "dm" } else { "audit" }), "Mark as False Positive")
                    .components(vec![CreateActionRow::InputText(
                        CreateInputText::new(InputTextStyle::Paragraph, "Why was this a false positive?", "reason")
                            .placeholder("Explain why you believe the bot incorrectly flagged this content...")
                            .max_length(1000)
                    )]);
                if let Err(error) = component.create_response(&ctx.http, CreateInteractionResponse::Modal(modal)).await {
                    warn!(%error, "Could not open false-positive modal");
                    self.send_error_dm(&ctx, &format!("Could not open false-positive modal: {error}")).await;
                }
            }
            Interaction::Modal(modal) if modal.data.custom_id.starts_with("aegis_fp_modal:") => {
                let mut id_parts = modal.data.custom_id.trim_start_matches("aegis_fp_modal:").split(':');
                let Ok(log_id) = id_parts.next().unwrap_or_default().parse::<u64>() else { return; };
                let is_dm_report = id_parts.next() == Some("dm");
                let already = self.false_positive_submitted.lock().map(|s| s.contains(&log_id)).unwrap_or(false);
                if already {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("⚠️ This false positive has already been submitted.").ephemeral(true);
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                }
                let reason = modal.data.components.iter().flat_map(|row| row.components.iter()).find_map(|component| {
                    if let ActionRowComponent::InputText(input) = component { input.value.clone() } else { None }
                }).unwrap_or_default();
                let data = self.false_positive_reports.lock().ok().and_then(|reports| reports.get(&log_id).cloned());
                let Some(data) = data else {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("❌ This detection report is no longer available.").ephemeral(true);
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                };
                let Some(channel_id_value) = self.config.false_positive_channel_id.as_deref() else {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("❌ `AEGIS_FALSE_POSITIVE_CHANNEL_ID` is not configured in `.env`.").ephemeral(true);
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                };
                let Ok(channel_id) = channel_id_value.parse::<u64>() else {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("❌ `AEGIS_FALSE_POSITIVE_CHANNEL_ID` in `.env` is invalid.").ephemeral(true);
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                };
                let report = CreateEmbed::new().title("🟡 False Positive Report")
                    .description("A scam detection was manually marked as a false positive.")
                    .timestamp(Timestamp::now())
                    .color(0xF1C40F)
                    .field("User", format!("{}\n`{}`", data.user_mention, data.user_id), false)
                    .field("Server", format!("{}\n`{}`", data.guild_name, data.guild_id), false)
                    .field("Channel", data.channel_name, false)
                    .field("Detection Reason", data.reason, false)
                    .field("Flagged Content", format!("```text\n{}\n```", data.flagged_content.chars().take(1000).collect::<String>()), false)
                    .field("Why was this a false positive?", reason.chars().take(1000).collect::<String>(), false)
                    .field("Marked By", format!("{}\n`{}`", modal.user.mention(), modal.user.id), false)
                    .field("Original Message", data.message_url, false);
                let owner_id = std::env::var("AEGIS_OWNER_ID").ok().and_then(|value| value.parse::<u64>().ok());
                let mut report_message = CreateMessage::new().content(owner_id.map(|id| format!("<@{id}>")).unwrap_or_default()).embed(report);
                if let Some(owner_id) = owner_id {
                    report_message = report_message.allowed_mentions(serenity::all::CreateAllowedMentions::new().users(vec![UserId::new(owner_id)]));
                }
                for (filename, bytes) in data.images {
                    report_message = report_message.add_file(CreateAttachment::bytes(bytes, filename));
                }
                let reserved = self.false_positive_submitted.lock()
                    .map(|mut submitted| submitted.insert(log_id)).unwrap_or(false);
                if !reserved {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("⚠️ This false positive has already been submitted.").ephemeral(true);
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                }
                match serenity::all::ChannelId::new(channel_id).send_message(&ctx.http, report_message).await {
                    Ok(_) => {
                        let report_insert = sqlx::query("INSERT OR IGNORE INTO false_positive_reports(case_id, reporter_id, report_reason, created_at, status) VALUES(?, ?, ?, ?, 'pending')")
                            .bind(log_id.to_string()).bind(modal.user.id.to_string())
                            .bind(reason.chars().take(1000).collect::<String>())
                            .bind(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
                            .execute(&self.analytics_db).await;
                        if let Err(error) = report_insert {
                            warn!(%error, case_id = %log_id, "Could not persist false-positive report");
                        }
                        let completed_button = CreateButton::new("aegis_false_positive")
                            .label("False Positive Submitted").emoji('✅')
                            .style(ButtonStyle::Secondary).disabled(true);
                        if let Ok(mut original) = modal.channel_id.message(&ctx.http, serenity::all::MessageId::new(log_id)).await {
                            let edit = EditMessage::new().components(vec![CreateActionRow::Buttons(vec![completed_button])]);
                            if let Err(error) = original.edit(&ctx.http, edit).await {
                                self.send_error_dm(&ctx, &format!("Could not update false-positive button: {error}")).await;
                            }
                        }
                        let report_settings = self.guild_settings(data.guild_id).await;
                        let confirmation = Self::render_template(&report_settings, "report",
                            "✅ False-positive report submitted.",
                            &[("user", data.user_mention.clone()), ("server", data.guild_name.clone()),
                              ("reason", reason.clone())], 1900)
                            .unwrap_or_else(|| "False-positive report submitted.".to_owned());
                        let response = serenity::all::CreateInteractionResponseMessage::new()
                            .content(confirmation).ephemeral(true);
                        let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    }
                    Err(error) => {
                        if let Ok(mut submitted) = self.false_positive_submitted.lock() { submitted.remove(&log_id); }
                        warn!(%error, "Could not send false-positive report");
                        self.send_error_dm(&ctx, &format!("Failed to send false-positive report: {error}")).await;
                        let response = serenity::all::CreateInteractionResponseMessage::new()
                            .content("❌ Failed to send the false-positive report.").ephemeral(true);
                        let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn message(&self, ctx: Context, message: Message) {
        let Some(message_guild_id) = message.guild_id else { return };
        self.record_analytics(message_guild_id.get(), 1, 0, None, 0, 0).await;
        if message.author.bot { return; }
        self.messages_since_heartbeat.fetch_add(1, Ordering::Relaxed);
        if message.content.starts_with("!ping") {
            let _ = message.channel_id.say(&ctx.http, "🏓 Pong!").await;
            return;
        }
        if message.content.split_whitespace().next() == Some("!stats") {
            if std::env::var("AEGIS_OWNER_ID").ok().and_then(|value| value.parse::<u64>().ok()) != Some(message.author.id.get()) { return; }
            let args: Vec<&str> = message.content.split_whitespace().collect();
            // Copy cached values into owned data so non-Send cache guards never cross an await.
            let mut guilds: Vec<(String, u64, Vec<u64>, serenity::all::GuildId)> =
                ctx.cache.guilds().into_iter().filter_map(|id| {
                    let guild = ctx.cache.guild(id)?;
                    Some((
                        guild.name.clone(),
                        guild.member_count,
                        guild.members.keys().map(|user_id| user_id.get()).collect(),
                        id,
                    ))
                }).collect();
            guilds.sort_by_key(|guild| guild.0.to_lowercase());
            if guilds.is_empty() {
                let _ = message.channel_id.say(&ctx.http, "I'm not in any servers.").await;
                return;
            }
            if args.len() > 2 {
                let _ = message.channel_id.say(&ctx.http, "Usage: `!stats [server_number]`").await;
                return;
            }
            if args.len() == 2 {
                let Ok(number) = args[1].parse::<usize>() else {
                    let _ = message.channel_id.say(&ctx.http, "Invalid server number.").await;
                    return;
                };
                if number == 0 || number > guilds.len() {
                    let _ = message.channel_id.say(&ctx.http, "Invalid server number.").await;
                    return;
                }
                let guild = &guilds[number - 1];
                let mut invite_url = None;
                if let Ok(channels) = guild.3.channels(&ctx.http).await {
                    for channel in channels.values() {
                        if let Ok(invite) = channel.create_invite(&ctx.http,
                            CreateInvite::new().max_age(0).max_uses(0).unique(false)).await {
                            invite_url = Some(invite.url());
                            break;
                        }
                    }
                }
                let invite_text = invite_url.unwrap_or_else(|| "❌ Could not create invite.".to_owned());
                let output = format!("**📊 Server Stats:**\n\n🏷️ Name: **{}**\n👥 Members: **{}**\n\n{}",
                    guild.0, guild.1, invite_text);
                let _ = message.channel_id.say(&ctx.http, output).await;
                return;
            }
            let mut users = HashSet::new();
            for guild in &guilds {
                for user_id in &guild.2 { users.insert(*user_id); }
            }
            let summary = format!("**🤖 Bot Statistics:**\n\n🌐 Servers: **{}**\n👥 Unique Users: **{}**",
                guilds.len(), users.len());
            let _ = message.channel_id.say(&ctx.http, summary).await;
            let lines: Vec<String> = guilds.iter().enumerate()
                .map(|(index, guild)| format!("`{}` {}", index + 1, guild.0))
                .collect();
            let mut chunks: Vec<String> = Vec::new();
            let mut current = String::new();
            for line in lines {
                if current.len() + line.len() + 1 > 1850 && !current.is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
                current.push_str(&line);
                current.push('\n');
            }
            if !current.is_empty() { chunks.push(current); }
            for (index, chunk) in chunks.iter().enumerate() {
                let output = if index == 0 { format!("**📋 Server List:**\n\n{chunk}") } else { chunk.clone() };
                let _ = message.channel_id.say(&ctx.http, output).await;
            }
            return;
        }
        if message.content.starts_with("!set-channel") {
            let Some(guild_id) = message.guild_id else { return; };
            let can_manage = match guild_id.member(&ctx.http, message.author.id).await {
                Ok(member) => ctx.cache.guild(guild_id)
                    .is_some_and(|guild| guild.member_permissions(&member).manage_channels()),
                Err(_) => false,
            };
            if !can_manage {
                let _ = message.channel_id.say(&ctx.http, "❌ You need the **Manage Channels** permission to use this command.").await;
                return;
            }
            let parts: Vec<&str> = message.content.split_whitespace().collect();
            if parts.len() != 3 || !matches!(parts[2], "logging") {
                let _ = message.channel_id.say(&ctx.http, "Usage: `!set-channel #channel logging`").await;
                return;
            }
            let channel_id = parts[1].trim_matches(|ch| ch == '<' || ch == '#' || ch == '>');
            let Ok(channel_id) = channel_id.parse::<u64>() else {
                let _ = message.channel_id.say(&ctx.http, "Please mention a valid channel.").await;
                return;
            };
            let target_id = serenity::all::ChannelId::new(channel_id);
            let channels = match guild_id.channels(&ctx.http).await {
                Ok(channels) => channels,
                Err(error) => {
                    warn!(%error, "Unable to list server channels for !set-channel");
                    let _ = message.channel_id.say(&ctx.http, "❌ Failed to find that channel.").await;
                    return;
                }
            };
            let Some(target) = channels.get(&target_id) else {
                let _ = message.channel_id.say(&ctx.http, "Please mention a text channel in this server.").await;
                return;
            };
            let mut topic = target.topic.clone().unwrap_or_default();
            if !topic.contains(&self.config.logging_identifier) {
                if !topic.is_empty() { topic.push('\n'); }
                topic.push_str(&self.config.logging_identifier);
                match target_id.edit(&ctx.http, serenity::all::EditChannel::new().topic(topic)).await {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(%error, "Unable to configure logging channel");
                        let _ = message.channel_id.say(&ctx.http, "❌ Failed to update the channel.").await;
                        return;
                    }
                }
            }
            let _ = message.channel_id.say(&ctx.http, format!("✅ <#{}> configured as the logging channel.", channel_id)).await;
            return;
        }
        let settings = self.guild_settings(message_guild_id.get()).await;
        if !Self::setting_bool(&settings, "enabled", true) { return; }
        if Self::setting_string_list_contains(&settings, "ignored_channel_ids", &message.channel_id.get().to_string()) { return; }

        let cached_member = ctx.cache.guild(message_guild_id)
            .and_then(|guild| guild.members.get(&message.author.id).cloned());
        if let Some(member) = cached_member.as_ref() {
            if Self::setting_bool(&settings, "exempt_admins", true)
                && ctx.cache.guild(message_guild_id).is_some_and(|guild| guild.member_permissions(member).administrator()) {
                return;
            }
            if settings.get("exempt_role_ids").and_then(serde_json::Value::as_array).is_some_and(|roles| {
                member.roles.iter().any(|role| {
                    let id = role.get().to_string();
                    roles.iter().any(|item| item.as_str() == Some(id.as_str()))
                })
            }) { return; }
        }

        let text_enabled = Self::setting_bool(&settings, "text_detection", true);
        let image_enabled = Self::setting_bool(&settings, "image_detection", true);
        let threshold = settings.get("delete_threshold").and_then(serde_json::Value::as_f64)
            .unwrap_or(self.config.delete_threshold).clamp(0.5, 1.0);
        let text_detection = if text_enabled { self.text_detector.scan(&message.content) } else { None };
        if let Some(detection) = text_detection.as_ref() {
            if detection.kind != DetectionKind::Campaign {
                info!(guild_id = ?message.guild_id, channel_id = %message.channel_id,
                    user_id = %message.author.id, "Aegis detected a text threat");
                self.handle_detection(&ctx, &message, &detection.reason, &detection.matched_text, Vec::new(), None, &settings).await;
                return;
            }
        }
        if !image_enabled { return; }
        let campaign_flagged = text_detection.as_ref().is_some_and(|d| d.reason == "discord_attachment_scam");
        let image_detection = self.scan_attachments(&ctx, &message, campaign_flagged, threshold).await;
        let Some((reason, bytes, confidence)) = image_detection else { return; };
        info!(guild_id = ?message.guild_id, channel_id = %message.channel_id,
            user_id = %message.author.id, "Aegis detected an image threat");
        self.handle_detection(&ctx, &message, "image_campaign", &reason, bytes, Some(confidence), &settings).await;
    }
}
