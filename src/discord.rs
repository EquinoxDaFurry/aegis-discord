use crate::{config::Config, detection::{image::ImageDetector, text::TextDetector, DetectionKind}};
use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serenity::gateway::ActivityData;
use serenity::{all::{ActionRowComponent, ButtonStyle, Context, CreateActionRow, CreateAttachment, CreateButton, CreateCommand, CreateEmbed, CreateInputText, CreateInteractionResponse, CreateInvite, CreateMessage, CreateModal, EditMessage, EventHandler, InputTextStyle, Interaction, Message, Timestamp, UserId}, prelude::Mentionable};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{Duration, SystemTime, UNIX_EPOCH}};
use tracing::{info, warn};

const DEBUG_USER_ID: u64 = 1326970244673310730;
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
            .bind(confidence.unwrap_or(0.0)).bind(i64::from(confidence.is_some()))
            .bind(deleted).bind(timeouts).execute(&self.analytics_db).await;
        if let Err(error) = result {
            warn!(%error, guild_id, "Could not update analytics aggregates");
        }
    }

    async fn send_error_dm(&self, ctx: &Context, error_text: &str) {
        if let Ok(user) = ctx.http.get_user(UserId::new(DEBUG_USER_ID)).await {
            let content = format!("⚠️ **Bot Error**\n\n```text\n{}
```", error_text.chars().take(1900).collect::<String>());
            let _ = user.dm(&ctx.http, CreateMessage::new().content(content)).await;
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

    async fn scan_attachments(&self, ctx: &Context, message: &Message, campaign_flagged: bool) -> Option<(String, Vec<(String, Vec<u8>)>, f64)> {
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
                tokio::task::spawn_blocking(move || detector.scan(&scan_bytes))).await;
            let scan_result = match scan_result {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => { warn!(%error, "Image scan worker failed"); continue; }
                Err(_) => { warn!(attachment = %attachment.filename, "Image scan timed out"); continue; }
            };
            match scan_result {
                Ok(Some(scan)) => {
                    if message.author.id.get() == DEBUG_USER_ID {
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

    async fn send_audit_log(&self, ctx: &Context, message: &Message, reason: &str, flagged_content: &str, deleted: bool, timeout_status: &str, images: Vec<(String, Vec<u8>)>, confidence: Option<f64>) {
        let Some(guild_id) = message.guild_id else { return };
        let channels = match guild_id.channels(&ctx.http).await {
            Ok(channels) => channels,
            Err(error) => { warn!(%error, "Could not inspect server channels for Aegis logging channel"); return; }
        };
        let target = channels.values().find(|channel| channel.topic.as_deref()
            .is_some_and(|topic| topic.contains(&self.config.logging_identifier)));
        let Some(channel) = target else { return };
        let safe_reason = flagged_content.chars().take(1000).collect::<String>().replace("```", "'''");
        let embed = CreateEmbed::new()
            .title("🚨 Scam Detected")
            .description("Possible scam content detected.")
            .timestamp(Timestamp::now())
            .color(0xED4245)
            .field("User", format!("{}\n`{}`", message.author.mention(), message.author.id), false)
            .field("Reason", reason, false)
            .field("Flagged Content", format!("```text\n{}\n```", safe_reason), false)
            .field("Channel", message.channel_id.mention().to_string(), true)
            .field("Deleted", deleted.to_string(), true)
            .field("Timeout", timeout_status, false);
        let button = CreateButton::new("aegis_false_positive")
            .label("False Positive").emoji('⚠').style(ButtonStyle::Secondary);
        let mut audit = CreateMessage::new().embed(embed)
            .components(vec![CreateActionRow::Buttons(vec![button])]);
        for (filename, bytes) in &images {
            audit = audit.add_file(CreateAttachment::bytes(bytes.clone(), filename.clone()));
        }
        match channel.send_message(&ctx.http, audit).await {
            Ok(log_message) => {
                if let Ok(mut reports) = self.false_positive_reports.lock() {
                    reports.insert(log_message.id.get(), FalsePositiveData {
                        user_mention: message.author.mention().to_string(),
                        user_id: message.author.id.get(),
                        guild_name: message.guild_id.and_then(|id| ctx.cache.guild(id).map(|g| g.name.clone())).unwrap_or_else(|| "Unknown server".to_owned()),
                        guild_id: guild_id.get(),
                        channel_name: message.channel_id.mention().to_string(),
                        reason: reason.chars().take(1000).collect(),
                        flagged_content: safe_reason,
                        message_url: message.link(),
                        images: images.clone(),
                    });
                }
            }
            Err(error) => {
                warn!(%error, "Could not send Aegis audit log");
                self.send_error_dm(ctx, &format!("Failed to send scam audit log: {error}")).await;
            }
        }
    }

    async fn handle_detection(&self, ctx: &Context, message: &Message, reason: &str, flagged_content: &str, images: Vec<(String, Vec<u8>)>) {
        self.threats_since_heartbeat.fetch_add(1, Ordering::Relaxed);
        let deleted = match message.delete(&ctx.http).await {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "Could not delete detected message (check Manage Messages permission)");
                self.send_error_dm(ctx, &format!("Could not delete detected message: {error}")).await;
                false
            }
        };
        let Some(guild_id) = message.guild_id else { return };
        self.record_analytics(guild_id.get(), 0, 1, confidence, i64::from(deleted), 0).await;
        if self.on_cooldown(guild_id.get(), message.author.id.get()) { return; }

        let timeout_status = match guild_id.member(&ctx.http, message.author.id).await {
            Ok(mut member) => {
                let until = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
                    .saturating_add(self.config.timeout_days.saturating_mul(86_400));
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
        };

        if timeout_status == "Timed out successfully" {
            self.record_analytics(guild_id.get(), 0, 0, None, 0, 1).await;
        }
        let guild_name = match guild_id.to_partial_guild(&ctx.http).await {
            Ok(guild) => guild.name,
            Err(_) => "Unknown server".to_owned(),
        };
        let flagged_text = flagged_content.chars().take(1500).collect::<String>();
        let dm_text = format!(
            "Hello <@{}>,\n\n⚠️ **Security Alert**\n\nYour account sent content commonly associated with scam campaigns.\n\nYour account may have been compromised.\n\nPlease:\n\n🔒 Reset your Discord password\n🛡️ Enable Two-Factor Authentication\n🦠 Run malware scans\n🔗 Review Authorized Apps\n\n\nYou have been timed out in:\n\n**{}**\n\nfor **{} days**.\n\n\nDetected location:\n\n<#{}>\n\n\nFlagged Content:\n\n```text\n{}\n```\n\n\nIf this was a mistake:\n\n{}\n\n\nPlease include:\n- The flagged message\n- What you intended to send\n- Any useful investigation details\n\n\n-# Anti-Scam Protection System",
            message.author.id, guild_name, self.config.timeout_days, message.channel_id, flagged_text.chars().take(1500).collect::<String>(), self.config.dm_report_server
        );
        let mut dm = CreateMessage::new().content(dm_text);
        for (filename, bytes) in &images {
            dm = dm.add_file(CreateAttachment::bytes(bytes.clone(), filename.clone()));
        }
        if let Err(error) = message.author.dm(&ctx.http, dm).await {
            warn!(%error, "Could not DM affected user");
            self.send_error_dm(ctx, &format!("Could not DM affected user: {error}")).await;
        }
        self.send_audit_log(ctx, message, reason, flagged_content, deleted, &timeout_status, images).await;
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
                        if let Ok(user) = ctx.http.get_user(UserId::new(DEBUG_USER_ID)).await {
                            let content = format!("⚠️ **Bot Error**\n\n```text\nWebsite heartbeat failed: {}\n```", error);
                            let _ = user.dm(&ctx.http, CreateMessage::new().content(content)).await;
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
            Interaction::Component(component) if component.data.custom_id == "aegis_false_positive" => {
                let log_id = component.message.id.get();
                let already = self.false_positive_submitted.lock().map(|s| s.contains(&log_id)).unwrap_or(false);
                if already {
                    let response = serenity::all::CreateInteractionResponseMessage::new()
                        .content("⚠️ This detection has already been marked as a false positive.").ephemeral(true);
                    let _ = component.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await;
                    return;
                }
                let modal = CreateModal::new(format!("aegis_fp_modal:{log_id}"), "Mark as False Positive")
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
                let id_text = modal.data.custom_id.trim_start_matches("aegis_fp_modal:");
                let Ok(log_id) = id_text.parse::<u64>() else { return; };
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
                let mut report_message = CreateMessage::new().embed(report);
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
                        let completed_button = CreateButton::new("aegis_false_positive")
                            .label("False Positive Submitted").emoji('✅')
                            .style(ButtonStyle::Secondary).disabled(true);
                        if let Ok(mut original) = modal.channel_id.message(&ctx.http, serenity::all::MessageId::new(log_id)).await {
                            let edit = EditMessage::new().components(vec![CreateActionRow::Buttons(vec![completed_button])]);
                            if let Err(error) = original.edit(&ctx.http, edit).await {
                                self.send_error_dm(&ctx, &format!("Could not update false-positive button: {error}")).await;
                            }
                        }
                        let response = serenity::all::CreateInteractionResponseMessage::new()
                            .content("✅ False-positive report submitted.").ephemeral(true);
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
            if message.author.id.get() != DEBUG_USER_ID { return; }
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
        let text_detection = self.text_detector.scan(&message.content);
        if let Some(detection) = text_detection.as_ref() {
            // Python treats CAMPAIGN as an image-context bonus, not a standalone text strike.
            if detection.kind != DetectionKind::Campaign {
                info!(guild_id = ?message.guild_id, channel_id = %message.channel_id,
                    user_id = %message.author.id, "Aegis detected a text threat");
                self.handle_detection(&ctx, &message, &detection.reason, &detection.matched_text, Vec::new(), None).await;
                return;
            }
        }
        let campaign_flagged = text_detection.as_ref().is_some_and(|d| d.reason == "discord_attachment_scam");
        let image_detection = self.scan_attachments(&ctx, &message, campaign_flagged).await;
        let Some((reason, bytes, confidence)) = image_detection else { return; };
        info!(guild_id = ?message.guild_id, channel_id = %message.channel_id,
            user_id = %message.author.id, "Aegis detected an image threat");
        self.handle_detection(&ctx, &message, "image_campaign", &reason, bytes, Some(confidence)).await;
    }
}
