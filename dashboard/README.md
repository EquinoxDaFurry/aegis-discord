# Aegis Oxide Dashboard

This directory contains the first dashboard UI prototype. It is intentionally separate from the existing `aegis-website` repository; the public marketing website is not modified.

## Open the preview

Open `index.html` in a browser, or serve this directory with any static file server.

## Current state

- Responsive left sidebar and server dashboard navigation.
- Overview, server configuration sections, and personal account placeholder.
- Per-server message-template editor with example placeholders and preview.
- Owner-only console concept with sample timeout history, search, status filters, and reversal-safety notes.
- Cross-Server Intelligence Network shown as a planned, not-yet-implemented feature.
- Existing Aegis logo and font styling referenced from the public website.

All displayed data is sample data. The editor, server picker, and owner controls are UI previews only. They do not authenticate users, persist configuration, query Discord, or reverse moderation actions.

## Planned Rust integration

The next implementation phase is to add an Axum API alongside the existing Serenity bot on Tokio, then connect:

1. Discord OAuth2 login and server membership/permission checks.
2. Server-isolated configuration records keyed by Discord guild ID.
3. Validated message templates and configuration updates.
4. Durable moderation case history and audited owner-only timeout reversals.
5. Bot-side configuration reload after successful persistence.

Owner authorization must be enforced server-side against a privately configured Discord user ID. Hiding the owner page in browser code is not a security boundary.