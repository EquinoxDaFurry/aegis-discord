# 🛡️ Aegis

> **Security shouldn't be a luxury.**

Aegis is an open defensive security system for Discord communities designed to detect, analyze, and respond to scam campaigns.

Aegis focuses on one principle:

> Build better protection. Share improvements. Keep communities safe.

Aegis allows people to learn from it, improve it, create alternatives, and build services around it while keeping core defensive protection accessible.

---

## ✨ Features

### 🔍 Text Scam Detection

Aegis uses a configurable rule-based detection engine to identify suspicious content.

Supported detection methods:

* Keyword detection
* Regular expression matching
* Scam campaign patterns
* Suspicious markdown links
* Fake visible URL detection

Rules can be updated without changing the bot code.

---

### 🖼️ Image Scam Detection

Aegis can identify known scam image campaigns using multiple hashing techniques.

Detection includes:

* SHA-256 exact image matching
* Perceptual hashing
* pHash comparison
* dHash comparison
* Average hash comparison
* Confidence-based scoring

This allows detection of reused scam images even when they have been resized or slightly modified.

---

### 🚨 Automatic Response

When a threat is detected, Aegis can:

* Delete the suspicious message
* Timeout the account
* Notify the affected user
* Provide account security recommendations
* Send incident information to moderators

Users receive guidance including:

* Resetting passwords
* Enabling 2FA
* Checking authorized applications
* Running malware scans

---

### 📋 Moderator Logging

Aegis provides detailed incident logs including:

* User information
* Detection reason
* Flagged content
* Channel location
* Message deletion status
* Timeout status

---

### 📊 Live Statistics

Aegis reports live protection statistics through heartbeat updates.

Tracked information includes:

* Protected servers
* Protected users
* Messages scanned
* Threats blocked
* Online status

View live stats:

https://aegisdiscord.pages.dev/

---

## ⚙️ How Detection Works

Aegis uses multiple layers of protection.

### Layer 1 — Text Analysis

Messages are scanned against configurable rules:

```
Message
   |
   v
Rule Engine
   |
   +--> Keyword checks
   |
   +--> Regex checks
   |
   +--> Link analysis
   |
   v
Threat result
```

---

### Layer 2 — Image Analysis

Image attachments are processed through:

```
Image
 |
 v
SHA-256 Check
 |
 +--> Exact match found
 |
 v
Image normalization
 |
 v
pHash / dHash / aHash
 |
 v
Similarity scoring
 |
 v
Confidence result
```

---

## 📈 Performance

Aegis includes several performance protections:

* Image download size limits
* Concurrent image scan limits
* Hash caching
* HTTP session reuse
* Detection cooldowns
* Heartbeat monitoring

---

## 🛠️ Installation

### Requirements

* Python 3.10+
* Discord bot application
* Required Python packages

Install dependencies:

```bash
pip install -r requirements.txt
```

---

### Configuration

Create a `.env` file:

```env
BOT=your_discord_token
WEBSITE_API=your_heartbeat_endpoint
API_KEY=your_api_key
```

Configure:

```
config.json
```

with your database, logging, timeout, and server settings.

---

## 🔒 Permissions

Aegis requires permissions for:

* Reading messages
* Reading message history
* Managing messages
* Moderating members
* Sending messages
* Sending files
* Embedding links

---

## 🤝 Contributing

Contributions are welcome.

You can:

* Improve detection systems
* Add new protections
* Improve performance
* Report security issues
* Create integrations

When contributing:

* Test your changes
* Avoid including secrets
* Respect user privacy
* Follow responsible disclosure practices

---

## 🛡️ Security Philosophy

Aegis follows a simple principle:

> Defensive security should protect people, not become a luxury product.

You are encouraged to:

* Learn from Aegis
* Fork Aegis
* Improve Aegis
* Create competing solutions
* Build services around Aegis

The goal is simple:

**More communities protected.**

---

## 💬 Support

For support, bug reports, or questions, join the Aegis Discord community.

Support server: https://discord.gg/yzuyuCGYm6

---

## 🛡️ Aegis

Protect communities.

Improve security.

Keep the internet safer.
