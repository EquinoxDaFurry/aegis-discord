# Aegis

> Scammers don't wait for your moderators.

Aegis is a Discord security bot designed to detect and respond to scams automatically.

Aegis provides an additional layer of protection for Discord communities by monitoring messages for common threats and responding when a threat is detected.

**[Add Aegis to your server](https://discord.com/oauth2/authorize?client_id=1529393411579183235)** · **[Website](https://aegisbot.pages.dev/)**

---

## What does Aegis do?

Aegis monitors your server for common scams and malicious content, allowing threats to be dealt with before they can spread through your community.

When Aegis detects a threat, it can automatically:

* Remove the malicious message
* Timeout the account responsible
* Notify the affected user through DMs
* Provide information to help the user secure their account

Aegis is designed to work alongside your moderation team, not replace it.

---

## Detection

Aegis currently uses deterministic detection techniques rather than AI or machine learning.

Detection includes:

* 🔗 Suspicious and malicious links
* 🖼️ Known scam imagery
* 🔤 Typosquatting
* 🔞 NSFW bait
* And other common Discord threats

For image-based threats, Aegis can use perceptual hashing techniques such as **pHash** and **dHash** to identify known malicious imagery.

---

## Threat Response

When a threat is detected, Aegis can take action automatically.

### 1. Detect

Aegis analyzes incoming content for known or suspicious threats.

### 2. Remove

The detected message is removed to prevent it from continuing to spread.

### 3. Timeout

The account responsible for the message can be timed out for **7 days** in the server.

This is intended to stop compromised accounts from continuing a scam campaign while their owner has time to secure the account.

### 4. Explain

The affected user receives a DM explaining what happened and providing guidance on securing their account.

---

## Privacy

Aegis stores **zero user data**.

Server owners control their own incident logging, including where logs are sent.

---

## Permissions

Aegis does **not** require the Administrator permission.

The bot currently uses permissions including:

* View Channels
* Send Messages
* Send Message in Threads
* Manage Messages
* Moderate Members
* Embed Links
* Attach Files
* Manage Channels
* View Message History

Giving Aegis Administrator permissions is optional, but may help prevent permission conflicts when the bot needs to perform moderation actions.

---

## Hosted Service

Aegis is primarily provided as a **hosted Discord bot**.

The source code is publicly available, but self-hosting instructions are intentionally not provided. If you want to use Aegis, please use the official hosted instance.

**[Add Aegis to your server](https://discord.com/oauth2/authorize?client_id=1529393411579183235)**

---

## Is Aegis free?

Yes.

Aegis is currently free to use, with no paid tiers or server/user limits.

---

## Development Status

Aegis is actively developed.

Features, detection methods, and responses may change as the project evolves.

---

## License

Aegis is licensed under the **GNU General Public License v3.0**.

See [`LICENSE`](LICENSE) for the full license text.

---

## Star History

<a href="https://www.star-history.com/?repos=EquinoxDaFurry%2Faegis-discord&type=timeline&legend=top-left">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=EquinoxDaFurry/aegis-discord&type=timeline&theme=dark&legend=top-left&sealed_token=ejcB9PpYuswW3soRIdYuOWKMWeH_rYyrZ1UOy5UW4lYzXqNfs603W31ysSqUhp-jWyVyQhUqaeikTU636v_RtjnOcx_Lt5EhXUuUXpeuzQyYHRwHFZxmO9Eg1VRSBt4Xp2cOBnSoRHqb9naFevCWBXSWjstguA0gDOtyYYgkY4h6c4YwkMmMGiB9v7wA>
  <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=EquinoxDaFurry/aegis-discord&type=timeline&legend=top-left&sealed_token=ejcB9PpYuswW3soRIdYuOWKMWeH_rYyrZ1UOy5UW4lYzXqNfs603W31ysSqUhp-jWyVyQhUqaeikTU636v_RtjnOcx_Lt5EhXUuUXpeuzQyYHRwHFZxmO9Eg1VRSBt4Xp2cOBnSoRHqb9naFevCWBXSWjstguA0gDOtyYYgkY4h6c4YwkMmMGiB9v7wA>
  <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=EquinoxDaFurry/aegis-discord&type=timeline&legend=top-left&sealed_token=ejcB9PpYuswW3soRIdYuOWKMWeH_rYyrZ1UOy5UW4lYzXqNfs603W31ysSqUhp-jWyVyQhUqaeikTU636v_RtjnOcx_Lt5EhXUuUXpeuzQyYHRwHFZxmO9Eg1VRSBt4Xp2cOBnSoRHqb9naFevCWBXSWjstguA0gDOtyYYgkY4h6c4YwkMmMGiB9v7wA>
</picture>

</a>
