# Aegis Oxide Dashboard

This directory contains the dashboard UI served by the Rust/Axum backend. It is part of the bot repository and does not modify the separate public aegis-website repository.

## Local setup

1. Create a Discord application in the Discord Developer Portal.
2. Add this exact OAuth2 redirect URL: http://127.0.0.1:8080/auth/callback
3. Set the dashboard OAuth values in your local .env file (never commit the client secret):
   AEGIS_DISCORD_CLIENT_ID=your_client_id
   AEGIS_DISCORD_CLIENT_SECRET=your_client_secret
   AEGIS_DISCORD_REDIRECT_URI=http://127.0.0.1:8080/auth/callback
   AEGIS_DASHBOARD_BIND=127.0.0.1:8080
4. Start Aegis from the repository root with cargo run.
5. Open http://127.0.0.1:8080.

The dashboard backend exposes /auth/login and /auth/callback. The dashboard UI intentionally does not show a separate Discord login button; the existing Aegis website is the intended entry point. Its login action must start the dashboard OAuth flow on this backend so the session cookie is issued for the dashboard origin. If the dashboard displays the sign-in prompt after returning, that handoff has not established a dashboard session.

Do not expose this early dashboard to the public internet. Production deployment still needs HTTPS cookie hardening, trusted-proxy configuration, rate limiting, and a security review.

## Current dashboard functionality

- **Server selector and overview:** loads the signed-in user's Discord guilds, separates manageable and view-only servers, and shows the selected server's live dashboard data.
- **Moderation:** reads Aegis-authored detection embeds from the selected server's configured logging channel, or discovers the channel through the configured logging identifier when no channel is selected. It shows successful timeout incidents and their available detection details.
- **Protection:** saves per-server detection toggles, image confidence threshold, deletion and timeout actions, timeout duration, dry-run mode, administrator and role exemptions, and ignored channels.
- **Messages:** saves per-server timeout, detection, and false-positive confirmation templates. The Rust bot reads these templates when sending the corresponding messages.
- **Logging:** saves one text channel per server. Clearing the selection returns the bot and moderation history reader to logging-identifier discovery.
- **Analytics:** records hourly aggregate message and detection counts, confidence samples, successful deletions, and successful timeouts. It does not store message content.

Settings are stored in the dashboard SQLite database and read by the running bot. Analytics history starts when tracking is active; earlier messages are not backfilled.

## Known limitations

- The owner console is intentionally not exposed. Owner-only cross-server actions and timeout reversals are not implemented in this dashboard phase.
- Moderation history is read from Discord's logging channel, limited to a 9-day lookback and at most 10,000 messages per scan. The interface warns if the scan reaches its limit.
- Historical timeout end states are inferred from the member's current Discord timeout and the configured duration; they are not a separately persisted per-incident record.
- OAuth guild permissions are cached for the one-hour dashboard session. Sign out and sign back in to refresh them after changing Discord permissions.
- Analytics confidence is based on image detections that produce a numeric confidence score.
- GitHub Actions runs Rust unit tests and compilation checks. Live Discord permissions, OAuth handoff, and end-to-end moderation still require testing in a controlled Discord server.
