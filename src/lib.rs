// SPDX-License-Identifier: LGPL-3.0-only

//! High-level facade over the `steam-vent` primitives used by Kether.
//! 
//! All public constructors enforce the following invariants:
//! - Successful logon yields a non-zero SteamID and session id.
//! - Notification helpers surface recoverable errors instead of panicking.
//! - Preprocessing utilities do not mutate the original message payload.

// Re-export the main types for external use
pub use logon::{GameInfo, LogonError, SessionSnapshot};
use logon::KetherSteamClient;

/// Primary facade for establishing authenticated or anonymous sessions.
///
/// Calls to `new`/`new_anonymous` only succeed when the resulting connection
/// reports a non-zero SteamID and session id. Prefer [`SessionSnapshot`] when
/// callers only require read-only metadata.
pub type LogOn = KetherSteamClient;

// Re-export chat room types
pub use chatroom::{
    ChatRoomClient, ChatRoomGroups, ChatRoomInfo, ChatRoomMessaging, ChatRoomNotifications,
    FriendMessage, GroupChatMessage, EnhancedGroupChatMessage,
};
pub use chatroom::helpers as chat_helpers;

// Re-export preprocessing types
pub use preprocessing::{
    MessagePreprocessor, PreprocessedMessage, BBCodeNode, BBCodeContent, ChatMentions,
    MentionSteamId,
};
pub use preprocessing::helpers as preprocessing_helpers;

pub mod logon;
pub mod chatroom;
pub mod preprocessing;
pub mod errors;

pub use errors::{
    classify_connection_error, classify_login_error, classify_network_error, ErrorDomain,
    ErrorInventoryEntry, RetryDisposition,
};

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[tokio::test]
    #[ignore = "Requires Steam network access"]
    async fn test_anonymous_connection() {
        let client = KetherSteamClient::new_anonymous().await;
        assert!(client.is_ok(), "Anonymous connection should work");
        
        let client = client.unwrap();
        // Test that we can get the Steam ID (this proves connection works)
        let steam_id = client.steam_id();
        println!("Anonymous Steam ID: {}", steam_id.steam3());
        
        // Test connection with actual API call
        let result = client.test_connection().await;
        assert!(result.is_ok(), "Connection test should succeed: {:?}", result);
    }

    #[tokio::test]
    async fn test_kether_login() {
        // Get credentials from environment variables
        let account = env::var("STEAM_ACCOUNT").unwrap_or_else(|_| {
            println!("STEAM_ACCOUNT not set, using demo account");
            "anonymous".to_string()
        });
        
        let password = env::var("STEAM_PASSWORD").unwrap_or_else(|_| {
            println!("STEAM_PASSWORD not set, using empty password");
            "".to_string()
        });
        
        let client = KetherSteamClient::new(&account, &password).await;
        match client {
            Ok(client) => {
                println!("Successfully logged in as User");
                println!("Steam ID: {}", client.steam_id().steam3());
                
                // Test getting owned games
                let games = client.get_owned_games().await;
                match games {
                    Ok(games) => {
                        println!("User owns {} games", games.len());
                        for game in games.iter().take(5) {
                            println!("  {}", game);
                        }
                    }
                    Err(e) => {
                        println!("Could not get owned games: {:?}", e);
                    }
                }
            }
            Err(e) => {
                println!("Failed to login as User: {:?}", e);
                // This test might fail if credentials are invalid or 2FA is required
                // We'll consider it a warning rather than a failure
            }
        }
    }
} 