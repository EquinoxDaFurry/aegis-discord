use anyhow::{Context, Result};
use std::{fs, path::Path};

#[derive(Clone, Debug)]
pub struct ImageHashRecord {
    pub sha256: String, pub phash: u64, pub dhash: u64, pub ahash: u64,
    pub filename: String, pub campaign: String,
}
#[derive(Clone, Debug, Default)]
pub struct RuleDatabase {
    pub words: Vec<String>, pub regex: Vec<String>, pub campaign_regex: Vec<String>,
    pub urls: Vec<String>, pub allow: Vec<String>, pub image_hashes: Vec<ImageHashRecord>,
}
impl RuleDatabase {
    pub fn load(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut db = Self::default();
        let mut section = "";
        let mut group = "";
        for (index, raw) in source.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if line == "[RULES]" { section = "RULES"; group = ""; continue; }
            if line == "[HASHES]" { section = "HASHES"; group = ""; continue; }
            if section == "RULES" {
                if let Some(name) = line.strip_suffix(':') { group = name; continue; }
                match group {
                    "WORDS" => db.words.push(line.to_owned()),
                    "REGEX" => db.regex.push(line.to_owned()),
                    "CAMPAIGN" => db.campaign_regex.push(line.to_owned()),
                    "URL" => db.urls.push(line.to_owned()),
                    "ALLOW" => db.allow.push(line.to_owned()),
                    _ => {}
                }
            } else if section == "HASHES" {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 5 {
                    tracing::warn!(line = index + 1, "Skipping malformed image-hash row");
                    continue;
                }
                let parsed = (|| -> Result<ImageHashRecord> {
                    Ok(ImageHashRecord {
                        sha256: parts[0].to_ascii_lowercase(),
                        phash: u64::from_str_radix(parts[1], 16)?,
                        dhash: u64::from_str_radix(parts[2], 16)?,
                        ahash: u64::from_str_radix(parts[3], 16)?,
                        filename: parts[4..].join(" "),
                        campaign: "unknown".to_owned(),
                    })
                })();
                match parsed {
                    Ok(record) => db.image_hashes.push(record),
                    Err(error) => tracing::warn!(line = index + 1, %error, "Skipping invalid hash row"),
                }
            }
        }
        Ok(db)
    }
    pub fn text_rule_count(&self) -> usize {
        self.words.len() + self.regex.len() + self.campaign_regex.len()
    }
    pub fn image_hash_count(&self) -> usize { self.image_hashes.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_aegis_database_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("database.aegis");
        fs::write(&path, "[RULES]\nWORDS:\nfree nitro\nREGEX:\n(?i)steam-gift\nCAMPAIGN:\nattachment scam\nURL:\nhttps://safe.example\nALLOW:\nhttps://discord.com\n[HASHES]\nabc 0123456789abcdef fedcba9876543210 0000000000000000 image.png\n").unwrap();
        let db = RuleDatabase::load(&path).unwrap();
        assert_eq!(db.words, vec!["free nitro"]);
        assert_eq!(db.regex.len(), 1);
        assert_eq!(db.campaign_regex.len(), 1);
        assert_eq!(db.urls.len(), 1);
        assert_eq!(db.allow.len(), 1);
        assert_eq!(db.image_hashes.len(), 1);
        assert_eq!(db.image_hashes[0].filename, "image.png");
    }
}
