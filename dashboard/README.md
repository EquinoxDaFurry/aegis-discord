# Aegis Oxide Dashboard

This directory contains the dashboard UI and is served by the Rust/Axum backend. It is separate from the existing `aegis-website` repository; the public marketing website is not modified.

## Local setup

1. Create a Discord application in the Discord Developer Portal.
2. Open its OAuth2 settings and add this exact redirect URL:
   `http://127.0.0.1:8080/auth/callback`
3. Copy the application's Client ID and Client Secret into the local `.env` file (never commit the secret):
   ```env
   AEGIS_DISCORD_CLIENT_ID=your_client_id
   AEGIS_DISCORD_CLIENT_SECRET=your_client_secret
   AEGIS_DISCORD_REDIRECT_URI=http://127.0.0.1:8080/auth/callback
   ```
4. Keep `AEGIS_DASHBOARD_BIND=127.0.0.1:8080` for local development.
5. Start Aegis from the repository root with `cargo run`, then open `http://127.0.0.1:8080`.
6. Choose **Sign in with Discord**. The dashboard requests only the `identify` and `guilds` OAuth scopes.

Do not expose this early dashboard to the public internet. Production deployment still needs HTTPS cookie hardening, trusted-proxy configuration, rate limiting, and a security review.

## Implemented in this phase

- Discord OAuth2 login and logout.
- Random one-time OAuth state stored as a hash and bound to a short-lived browser cookie.
- One-hour dashboard sessions; only a hash of the session token is stored in SQLite.
- Server list limited to guilds where the signed-in user is the owner or has Manage Server / Administrator permission.
- Per-guild settings read/write API, with server-side permission checks and input validation.
- SQLite persistence for enabled state, default timeout duration, image confidence threshold, and logging channel ID.
- Responsive dashboard settings form connected to the Rust API.

## Current limitations

- These settings are persisted per server, but are not yet applied to the live moderation pipeline. Bot-side config loading/reload is the next integration step.
- Message templates, moderation history, analytics, and owner-only timeout reversal remain previews and are not wired to live data.
- OAuth guild permissions are cached for the one-hour session. Sign out and sign back in to refresh the list after changing Discord permissions.
- The dashboard should remain bound to localhost until the production security work is complete.
