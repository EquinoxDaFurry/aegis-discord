pub mod database;
pub mod image;
pub mod text;
pub use database::RuleDatabase;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectionKind { Wordlist, Regex, FakeVisibleLink, Campaign }

#[derive(Clone, Debug)]
pub struct Detection {
    pub kind: DetectionKind,
    pub reason: String,
    pub matched_text: String,
}
impl Detection {
    pub fn new(kind: DetectionKind, reason: impl Into<String>, matched_text: impl Into<String>) -> Self {
        Self { kind, reason: reason.into(), matched_text: matched_text.into() }
    }
}
