# Aegis Oxide

> Rust-powered scam detection for Discord. Aegis, with a little more bite.

**Aegis Oxide** is Aegis 2.0: the Rust rewrite of my Discord anti-scam bot!

Aegis watches messages and image attachments for known scam patterns, then helps moderators respond before the same bait gets passed around the server.

## What it checks

Aegis uses a local rules and image-hash database. It isn't an AI moderation model; detentions come from the rules and hashes in that database.

- **Text rules:** word-list matches, regular expressions (including common lookarounds and backreferences), and campaign patterns.
- **Fake links:** catches Markdown links whose visible text impersonates familiar services while pointing somewhere else.
- **Images:** checks known image hashes and perceptual similarity, so altered copies of known scam images can still be recognized.
- **Repeat scans:** caches image-hash calculations and limits concurrent image scans to keep the bot's workload under control.

Detection depends on the contents and quality of the database. No filter catches everything, and a detection can be wrong.

## When something gets flagged

Depending on Discord permissions and the server configuration, Oxide can:

1. Delete the detected message.
2. Apply the configured member timeout.
3. DM the affected account with a security notice and next steps.
4. Send a moderation log to the channel identified by the configured topic marker.
5. Let moderators submit a false-positive report from the log message.

The bot needs the relevant permissions to carry out each action. If Discord denies an action, Aegis can't force it through.

## Commands

| Command | What it does |
| --- | --- |
| `/help` | Shows the help command. |
| `!ping` | Checks that the bot responds. |
| `!stats` | Shows diagnostic server statistics (owner/debug use). |
| `!stats <number>` | Shows details for a server in the stats list. |
| `!set-channel #channel logging` | Sets the logging topic marker for a channel. |

Useful settings:

| Variable | Purpose | Default |
| --- | --- | --- |
| `BOT` | Discord bot token | — |
| `AEGIS_DATABASE` | Detection database path | `database.aegis` |
| `AEGIS_DELETE_THRESHOLD` | Minimum image-match confidence | `0.805` |
| `AEGIS_TIMEOUT_DAYS` | Member timeout duration | `7` |
| `AEGIS_COOLDOWN_SECONDS` | Per-user incident cooldown | `900` |
| `AEGIS_MAX_IMAGE_BYTES` | Maximum image attachment size to scan | `3145728` |
| `AEGIS_IMAGE_CACHE_ENTRIES` | Maximum cached image-hash results | `5000` |
| `AEGIS_DM_REPORT_SERVER` | Text shown as the false-positive/report destination in the user DM | `Not configured` |
| `AEGIS_LOGGING_IDENTIFIER` | Marker searched for in a channel's topic | `.eq.antiscam.logs` |
| `AEGIS_FALSE_POSITIVE_CHANNEL_ID` | Channel for false-positive reports | unset |
| `WEBSITE_API`, `API_KEY` | Optional authenticated website heartbeat | unset |

## Build it

You'll need a stable Rust toolchain.

```sh
cargo test
cargo build --release
```

The release binary is written to `target/release/aegis`.

Build on the same operating system and architecture as the machine that will run it. For a Linux host, build on Linux — a Windows `.exe` won't run in a Linux container.

## A couple of practical notes

- Enable the Discord **Message Content** intent for the application; Oxide also requests the guild, message, and member intents it uses.
- Give the bot only the permissions it needs, including Manage Messages and Moderate Members for deletion and timeouts, plus the channel permissions needed to send logs and DMs.
- Keep the detection database up to date. Aegis can only recognise patterns represented by its rules and image hashes.
- False-positive report state is held in memory, so pending reports are not a permanent record across restarts.

## License

Aegis Oxide is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE).
