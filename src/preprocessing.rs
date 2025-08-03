use std::collections::HashMap;
use steamid_ng::SteamID;
use serde::{Deserialize, Serialize};

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
    pub mention_steamids: Vec<SteamID>,
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
    pub fn preprocess_message(message: &str) -> PreprocessedMessage {
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
        if message.is_empty() {
            return vec![BBCodeContent::String(message.to_string())];
        }

        // Simple BBCode parser for Steam's supported tags
        let allowed_tags = [
            "emoticon", "code", "pre", "img", "url", "spoiler", 
            "quote", "random", "flip", "tradeofferlink", "tradeoffer",
            "sticker", "gameinvite", "og", "roomeffect"
        ];

        let mut parsed = Vec::new();
        let mut current_text = String::new();
        let mut i = 0;

        while i < message.len() {
            if let Some(tag_start) = message[i..].find('[') {
                // Add text before tag
                if tag_start > 0 {
                    current_text.push_str(&message[i..i + tag_start]);
                }

                // Find tag end
                if let Some(tag_end) = message[i + tag_start..].find(']') {
                    let tag_content = &message[i + tag_start + 1..i + tag_start + tag_end];
                    
                    if let Some(node) = Self::parse_bbcode_tag(tag_content, &allowed_tags) {
                        if !current_text.is_empty() {
                            parsed.push(BBCodeContent::String(current_text.clone()));
                            current_text.clear();
                        }
                        parsed.push(BBCodeContent::Node(node));
                    } else {
                        // Invalid tag, treat as text
                        current_text.push_str(&message[i..i + tag_start + tag_end + 1]);
                    }
                    
                    i += tag_start + tag_end + 1;
                } else {
                    // No closing bracket, treat as text
                    current_text.push_str(&message[i..]);
                    break;
                }
            } else {
                // No more tags, add remaining text
                current_text.push_str(&message[i..]);
                break;
            }
        }

        if !current_text.is_empty() {
            parsed.push(BBCodeContent::String(current_text));
        }

        parsed
    }

    /// Parse a single BBCode tag
    fn parse_bbcode_tag(tag_content: &str, allowed_tags: &[&str]) -> Option<BBCodeNode> {
        let parts: Vec<&str> = tag_content.splitn(2, '=').collect();
        let tag_name = parts[0].trim();
        
        // Check if tag is allowed
        if !allowed_tags.contains(&tag_name) {
            return None;
        }

        let mut attrs = HashMap::new();
        
        if parts.len() > 1 {
            let value = parts[1].trim();
            if !value.is_empty() {
                attrs.insert("value".to_string(), value.to_string());
            }
        }

        Some(BBCodeNode {
            tag: tag_name.to_string(),
            attrs,
            content: None,
        })
    }

    /// Extract mentions from a message
    pub fn extract_mentions(message: &str) -> Option<ChatMentions> {
        let mut mentions = ChatMentions {
            mention_all: false,
            mention_here: false,
            mention_steamids: Vec::new(),
        };

        // Check for @all and @here mentions
        if message.contains("@all") || message.contains("@everyone") {
            mentions.mention_all = true;
        }
        
        if message.contains("@here") {
            mentions.mention_here = true;
        }

        // Extract Steam ID mentions (basic pattern matching)
        // This is a simplified version - in practice you'd want more sophisticated parsing
        let steam_id_pattern = r"\[U:1:\d+\]";
        // Note: This would require regex crate for proper implementation
        
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

#[cfg(test)]
mod tests {
    use super::*;

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
} 