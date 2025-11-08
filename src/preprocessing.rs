// SPDX-License-Identifier: LGPL-3.0-only

use std::collections::HashMap;
use tracing::trace;
use steamid_ng::SteamID;
use serde::{Deserialize, Serialize};

const ALLOWED_BBCODE_TAGS: &[&str] = &[
    "emoticon", "code", "pre", "img", "url", "spoiler", "quote", "random", "flip",
    "tradeofferlink", "tradeoffer", "sticker", "gameinvite", "og", "roomeffect",
];

/// Represents a BBCode node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BBCodeNode {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub content: Option<Vec<BBCodeContent>>,
}

/// Represents BBCode content (either string or node)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BBCodeContent {
    String(String),
    Node(BBCodeNode),
}

/// Represents chat mentions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMentions {
    pub mention_all: bool,
    pub mention_here: bool,
    pub mention_steamids: Vec<MentionSteamId>,
}

/// Wrapper around `SteamID` that supports serde serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MentionSteamId(pub SteamID);

impl MentionSteamId {
    pub fn into_inner(self) -> SteamID {
        self.0
    }

    pub fn as_inner(&self) -> SteamID {
        self.0
    }
}

impl From<SteamID> for MentionSteamId {
    fn from(value: SteamID) -> Self {
        MentionSteamId(value)
    }
}

impl From<MentionSteamId> for SteamID {
    fn from(value: MentionSteamId) -> Self {
        value.0
    }
}

impl serde::Serialize for MentionSteamId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(u64::from(self.0))
    }
}

impl<'de> serde::Deserialize<'de> for MentionSteamId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u64::deserialize(deserializer)?;
        Ok(MentionSteamId(SteamID::from(raw)))
    }
}

/// Preprocessed message with BBCode parsing and mentions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreprocessedMessage {
    pub original_message: String,
    pub modified_message: String,
    pub message_bbcode_parsed: Vec<BBCodeContent>,
    pub mentions: Option<ChatMentions>,
    pub server_timestamp: Option<u32>,
    pub ordinal: Option<u32>,
}

/// Message preprocessor for Steam chat
pub struct MessagePreprocessor;

impl MessagePreprocessor {
    /// Preprocess a message with BBCode parsing and mention detection
    #[tracing::instrument(name = "kether.preprocess.message", skip(message))]
    pub fn preprocess_message(message: &str) -> PreprocessedMessage {
        trace!(original_len = message.len(), "starting preprocessing");
        let message_bbcode_parsed = Self::parse_bbcode(message);
        let mentions = Self::extract_mentions(message);
        
        PreprocessedMessage {
            original_message: message.to_string(),
            modified_message: message.to_string(),
            message_bbcode_parsed,
            mentions,
            server_timestamp: None,
            ordinal: None,
        }
    }

    /// Parse BBCode from a message string
    pub fn parse_bbcode(message: &str) -> Vec<BBCodeContent> {
        bbcode::Parser::new(ALLOWED_BBCODE_TAGS).parse(message)
    }

    /// Extract mentions from a message
    pub fn extract_mentions(message: &str) -> Option<ChatMentions> {
        let mut mentions = ChatMentions {
            mention_all: false,
            mention_here: false,
            mention_steamids: Vec::new(),
        };

        for raw_token in message.split_whitespace() {
            let token = raw_token.trim_matches(|c: char| "!?,.;".contains(c));
            match token {
                "@all" | "@everyone" => mentions.mention_all = true,
                "@here" => mentions.mention_here = true,
                _ if token.starts_with("[U:1:") && token.ends_with(']') => {
                    if let Ok(steam_id) = SteamID::try_from(token) {
                        mentions.mention_steamids.push(MentionSteamId::from(steam_id));
                    }
                }
                _ => {}
            }
        }

        if mentions.mention_all || mentions.mention_here || !mentions.mention_steamids.is_empty() {
            Some(mentions)
        } else {
            None
        }
    }

    /// Convert a message with mentions to a format suitable for sending
    pub fn prepare_message_for_sending(message: &str) -> String {
        // Remove or escape special characters that might cause issues
        let mut prepared = message.to_string();
        
        // Basic escaping - in practice you'd want more sophisticated handling
        prepared = prepared.replace("\\[", "[").replace("\\]", "]");
        
        prepared
    }

    /// Process a response from Steam with preprocessing
    pub fn process_response(
        original_message: &str,
        modified_message: &str,
        server_timestamp: u32,
        ordinal: u32,
    ) -> PreprocessedMessage {
        let message_bbcode_parsed = Self::parse_bbcode(modified_message);
        let mentions = Self::extract_mentions(modified_message);

        PreprocessedMessage {
            original_message: original_message.to_string(),
            modified_message: modified_message.to_string(),
            message_bbcode_parsed,
            mentions,
            server_timestamp: Some(server_timestamp),
            ordinal: Some(ordinal),
        }
    }
}

/// Helper functions for message processing
pub mod helpers {
    use super::*;

    /// Create a simple mention for a Steam ID
    pub fn create_mention(steam_id: SteamID) -> String {
        format!("@{}", steam_id.steam3())
    }

    /// Create an @all mention
    pub fn create_all_mention() -> String {
        "@all".to_string()
    }

    /// Create an @here mention
    pub fn create_here_mention() -> String {
        "@here".to_string()
    }

    /// Check if a message contains any mentions
    pub fn has_mentions(message: &str) -> bool {
        message.contains("@all") || message.contains("@here") || message.contains("@")
    }

    /// Format a message with BBCode
    pub fn format_with_bbcode(message: &str, bbcode_type: &str, value: &str) -> String {
        match bbcode_type {
            "bold" => format!("[b]{}[/b]", message),
            "italic" => format!("[i]{}[/i]", message),
            "underline" => format!("[u]{}[/u]", message),
            "spoiler" => format!("[spoiler]{}[/spoiler]", message),
            "code" => format!("[code]{}[/code]", message),
            "url" => format!("[url={}]{}[/url]", value, message),
            "emoticon" => format!("[emoticon:{}]", value),
            _ => message.to_string(),
        }
    }
}

mod bbcode {
    use super::{BBCodeContent, BBCodeNode};
    use std::collections::HashMap;

    pub struct Parser<'a> {
        allowed_tags: &'a [&'a str],
    }

    impl<'a> Parser<'a> {
        pub fn new(allowed_tags: &'a [&'a str]) -> Self {
            Self { allowed_tags }
        }

        pub fn parse(&self, message: &str) -> Vec<BBCodeContent> {
            if message.is_empty() {
                return vec![BBCodeContent::String(message.to_string())];
            }

            let mut parsed = Vec::new();
            let mut current_text = String::new();
            let mut i = 0;

            while i < message.len() {
                if let Some(tag_start) = message[i..].find('[') {
                    if tag_start > 0 {
                        current_text.push_str(&message[i..i + tag_start]);
                    }

                    if let Some(tag_end) = message[i + tag_start..].find(']') {
                        let tag_content =
                            &message[i + tag_start + 1..i + tag_start + tag_end];

                        if let Some(node) = self.parse_tag(tag_content) {
                            if !current_text.is_empty() {
                                parsed.push(BBCodeContent::String(std::mem::take(
                                    &mut current_text,
                                )));
                            }
                            parsed.push(BBCodeContent::Node(node));
                        } else {
                            current_text
                                .push_str(&message[i..i + tag_start + tag_end + 1]);
                        }

                        i += tag_start + tag_end + 1;
                    } else {
                        current_text.push_str(&message[i..]);
                        break;
                    }
                } else {
                    current_text.push_str(&message[i..]);
                    break;
                }
            }

            if !current_text.is_empty() {
                parsed.push(BBCodeContent::String(current_text));
            }

            parsed
        }

        fn parse_tag(&self, tag_content: &str) -> Option<BBCodeNode> {
            let parts: Vec<&str> = tag_content.splitn(2, '=').collect();
            let tag_name = parts[0].trim();

            if !self.allowed_tags.contains(&tag_name) {
                return None;
            }

            let mut attrs = HashMap::new();

            if let Some(value) = parts
                .get(1)
                .map(|s| s.trim())
                .filter(|value| !value.is_empty())
            {
                attrs.insert("value".to_string(), value.to_string());
            }

            Some(BBCodeNode {
                tag: tag_name.to_string(),
                attrs,
                content: None,
            })
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_bbcode_parsing() {
        let message = "Hello [b]world[/b] and [i]italic[/i] text";
        let parsed = MessagePreprocessor::parse_bbcode(message);
        
        assert!(!parsed.is_empty());
        // Basic test - in practice you'd want more detailed assertions
    }

    #[test]
    fn test_mention_extraction() {
        let message = "Hello @all and @here users!";
        let mentions = MessagePreprocessor::extract_mentions(message);
        
        assert!(mentions.is_some());
        let mentions = mentions.unwrap();
        assert!(mentions.mention_all);
        assert!(mentions.mention_here);
    }

    #[test]
    fn test_message_preprocessing() {
        let message = "Hello @all with [b]bold[/b] text";
        let preprocessed = MessagePreprocessor::preprocess_message(message);
        
        assert_eq!(preprocessed.original_message, message);
        assert!(preprocessed.mentions.is_some());
        assert!(!preprocessed.message_bbcode_parsed.is_empty());
    }

    #[test]
    fn test_prepare_message_for_sending_preserves_brackets() {
        let message = r"Look at \[b\]escaped brackets\[/b\]";
        let prepared = MessagePreprocessor::prepare_message_for_sending(message);
        assert_eq!(prepared, "Look at [b]escaped brackets[/b]");
    }

    #[test]
    fn test_process_response_round_trips_metadata() {
        let original = "hello";
        let modified = "hello world";
        let processed = MessagePreprocessor::process_response(original, modified, 42, 7);

        assert_eq!(processed.original_message, original);
        assert_eq!(processed.modified_message, modified);
        assert_eq!(processed.server_timestamp, Some(42));
        assert_eq!(processed.ordinal, Some(7));
    }

    #[test]
    fn test_bbcode_parser_rejects_unknown_tags() {
        let parser = bbcode::Parser::new(&["b"]);
        let parsed = parser.parse("[unknown]value[/unknown]");

        assert_eq!(parsed.len(), 1);
        assert!(matches!(parsed[0], BBCodeContent::String(_)));
    }

    #[test]
    fn test_mentions_roundtrip_serialization() {
        let steam_id = SteamID::from(42u64);
        let mentions = ChatMentions {
            mention_all: true,
            mention_here: false,
            mention_steamids: vec![MentionSteamId::from(steam_id)],
        };

        let json = serde_json::to_string(&mentions).expect("serialize mentions");
        let decoded: ChatMentions =
            serde_json::from_str(&json).expect("deserialize mentions");

        assert_eq!(decoded.mention_steamids.len(), 1);
        assert_eq!(SteamID::from(decoded.mention_steamids[0]), steam_id);
    }

    #[test]
    fn test_preprocessing_edge_cases() {
        struct Case<'a> {
            name: &'a str,
            message: &'a str,
            min_node_count: usize,
            expect_all: bool,
            expect_here: bool,
            expected_steam_ids: usize,
        }

        let cases = [
            Case {
                name: "nested_bbcode",
                message: "Nested [spoiler]outer [code]inner[/code][/spoiler] tags",
                min_node_count: 1,
                expect_all: false,
                expect_here: false,
                expected_steam_ids: 0,
            },
            Case {
                name: "invalid_mention_inside_word",
                message: "email@all.com should not ping everyone",
                min_node_count: 0,
                expect_all: false,
                expect_here: false,
                expected_steam_ids: 0,
            },
            Case {
                name: "multilingual_mentions",
                message: "こんにちは @here друзья [U:1:1531059355]",
                min_node_count: 0,
                expect_all: false,
                expect_here: true,
                expected_steam_ids: 1,
            },
        ];

        for case in cases {
            let preprocessed = MessagePreprocessor::preprocess_message(case.message);
            let node_count = preprocessed
                .message_bbcode_parsed
                .iter()
                .filter(|content| matches!(content, BBCodeContent::Node(_)))
                .count();
            assert!(node_count >= case.min_node_count, "{}", case.name);

            match preprocessed.mentions {
                Some(ref mentions) => {
                    assert_eq!(mentions.mention_all, case.expect_all, "{}", case.name);
                    assert_eq!(mentions.mention_here, case.expect_here, "{}", case.name);
                    assert_eq!(
                        mentions.mention_steamids.len(),
                        case.expected_steam_ids,
                        "{}",
                        case.name
                    );
                }
                None => {
                    assert!(
                        !case.expect_all && !case.expect_here && case.expected_steam_ids == 0,
                        "{} expected mentions",
                        case.name
                    );
                }
            }
        }
    }
} 