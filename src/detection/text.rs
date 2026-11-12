use anyhow::Result;
use fancy_regex::{Regex as PythonRegex, RegexBuilder as PythonRegexBuilder};
use regex::Regex as SimpleRegex;
use super::{database::RuleDatabase, Detection, DetectionKind};

/// Python's `re` supports backtracking constructs (lookarounds and backreferences)
/// that Rust's standard `regex` crate intentionally does not. Use fancy-regex for
/// database rules, while retaining the fast linear-time engine for Markdown parsing.
pub struct TextDetector {
    words: Vec<String>,
    regex: Vec<PythonRegex>,
    campaign_regex: Vec<PythonRegex>,
    allow: Vec<String>,
    markdown_link: SimpleRegex,
}

impl TextDetector {
    pub fn from_rules(db: &RuleDatabase) -> Result<Self> {
        let compile = |patterns: &[String]| -> Result<Vec<PythonRegex>> {
            patterns.iter().map(|pattern| {
                PythonRegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .map_err(|error| anyhow::anyhow!("Invalid Aegis regex rule {pattern:?}: {error}"))
            }).collect()
        };

        Ok(Self {
            words: db.words.iter().map(|word| word.to_lowercase()).collect(),
            regex: compile(&db.regex)?,
            campaign_regex: compile(&db.campaign_regex)?,
            allow: db.allow.clone(),
            markdown_link: SimpleRegex::new(r"\[([^\]]+)\]\((https?://[^)]+)\)")?,
        })
    }

    fn is_allowed_url(&self, url: &str) -> bool {
        self.allow.iter().any(|allowed| url.starts_with(allowed))
    }

    pub fn scan(&self, message: &str) -> Option<Detection> {
        let lower = message.to_lowercase();

        for word in &self.words {
            if !word.is_empty() && lower.contains(word) {
                return Some(Detection::new(DetectionKind::Wordlist, "wordlist", word.clone()));
            }
        }

        // Python re.search() returns the first match for each rule. fancy-regex
        // supports lookahead/lookbehind, backreferences and named capture groups.
        for pattern in &self.regex {
            match pattern.find(message) {
                Ok(Some(found)) => return Some(Detection::new(
                    DetectionKind::Regex, "regex", found.as_str()
                )),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Aegis regex rule exceeded its runtime limits"),
            }
        }

        let fake_brands = ["steamcommunity.com", "roblox.com", "discord.com", "discord.gg"];
        for captures in self.markdown_link.captures_iter(message) {
            let visible = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            let destination = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
            if fake_brands.iter().any(|brand| visible.to_lowercase().contains(brand))
                && !self.is_allowed_url(destination) {
                return Some(Detection::new(
                    DetectionKind::FakeVisibleLink, "fake_visible_link", visible
                ));
            }
        }

        for pattern in &self.campaign_regex {
            match pattern.find(message) {
                Ok(Some(found)) => return Some(Detection::new(
                    DetectionKind::Campaign, "discord_attachment_scam", found.as_str()
                )),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Aegis campaign regex rule exceeded its runtime limits"),
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wordlist_and_fake_markdown_links() {
        let db = RuleDatabase {
            words: vec!["free nitro".into()],
            allow: vec!["https://discord.com/".into()],
            ..Default::default()
        };
        let detector = TextDetector::from_rules(&db).unwrap();
        assert_eq!(detector.scan("claim FREE NITRO").unwrap().reason, "wordlist");
        assert_eq!(detector.scan("[discord.com](https://evil.example/login)").unwrap().reason, "fake_visible_link");
        assert!(detector.scan("[discord.com](https://discord.com/safety)").is_none());
    }

    #[test]
    fn supports_python_lookahead_and_lookbehind() {
        let db = RuleDatabase {
            regex: vec![r"(?<=claim )free(?= nitro)".into()],
            ..Default::default()
        };
        let detector = TextDetector::from_rules(&db).unwrap();
        assert_eq!(detector.scan("please claim free nitro now").unwrap().matched_text, "free");
    }

    #[test]
    fn supports_python_backreferences() {
        let db = RuleDatabase {
            regex: vec![r"(?i)\b(steam|discord)\s+\1\b".into()],
            ..Default::default()
        };
        let detector = TextDetector::from_rules(&db).unwrap();
        assert_eq!(detector.scan("STEAM steam").unwrap().matched_text, "STEAM steam");
    }

    #[test]
    fn supports_python_named_capture_groups() {
        let db = RuleDatabase {
            regex: vec![r"(?P<invite>discord\.gg/[A-Za-z0-9]+)".into()],
            ..Default::default()
        };
        let detector = TextDetector::from_rules(&db).unwrap();
        assert_eq!(detector.scan("join discord.gg/abc123").unwrap().matched_text, "discord.gg/abc123");
    }

    #[test]
    fn campaign_regex_supports_lookarounds() {
        let db = RuleDatabase {
            campaign_regex: vec![r"(?i)free(?=\s+nitro)".into()],
            ..Default::default()
        };
        let detector = TextDetector::from_rules(&db).unwrap();
        let detection = detector.scan("free nitro here").unwrap();
        assert_eq!(detection.kind, DetectionKind::Campaign);
        assert_eq!(detection.matched_text, "free");
    }

    #[test]
    fn rejects_invalid_regex_with_rule_context() {
        let db = RuleDatabase {
            regex: vec!["(".into()],
            ..Default::default()
        };
        let error = match TextDetector::from_rules(&db) {
            Ok(_) => panic!("invalid regex unexpectedly compiled"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Invalid Aegis regex rule"));
    }
}
