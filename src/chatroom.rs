// SPDX-License-Identifier: LGPL-3.0-only

use std::error::Error;
use steam_vent::proto::steammessages_chat_steamclient::{
    CChatRoom_GetMyChatRoomGroups_Request,
    CChatRoom_GetMyChatRoomGroups_Response,
    CChatRoom_JoinChatRoomGroup_Request,
    CChatRoom_JoinChatRoomGroup_Response,
    CChatRoom_LeaveChatRoomGroup_Request,
    CChatRoom_LeaveChatRoomGroup_Response,
    CChatRoom_SendChatMessage_Request,
    CChatRoom_SendChatMessage_Response,
    CChatRoom_IncomingChatMessage_Notification,
    CChatRoom_GetChatRoomGroupState_Request,
    CChatRoom_GetChatRoomGroupState_Response,
};
use steam_vent::proto::steammessages_friendmessages_steamclient::{
    CFriendMessages_SendMessage_Request,
    CFriendMessages_SendMessage_Response,
    CFriendMessages_IncomingMessage_Notification,
};
use steam_vent::ConnectionTrait;
use steamid_ng::SteamID;
use tokio_stream::StreamExt;
use crate::preprocessing::{MessagePreprocessor, PreprocessedMessage};

/// Chat room information
#[derive(Debug, Clone)]
pub struct ChatRoomInfo {
    pub chat_group_id: u64,
    pub chat_id: u64,
    pub chat_name: String,
    pub chat_group_name: String,
    pub is_joined: bool,
}

/// Friend message information
#[derive(Debug, Clone)]
pub struct FriendMessage {
    pub steam_id: SteamID,
    pub message: String,
    pub timestamp: u32,
    pub chat_entry_type: i32,
}

/// Group chat message information
#[derive(Debug, Clone)]
pub struct GroupChatMessage {
    pub chat_group_id: u64,
    pub chat_id: u64,
    pub sender_steam_id: SteamID,
    pub message: String,
    pub timestamp: u32,
    pub chat_name: String,
    pub ordinal: u32,
}

/// Enhanced group chat message with preprocessing
#[derive(Debug, Clone)]
pub struct EnhancedGroupChatMessage {
    pub chat_group_id: u64,
    pub chat_id: u64,
    pub sender_steam_id: SteamID,
    pub message: String,
    pub timestamp: u32,
    pub chat_name: String,
    pub ordinal: u32,
    pub preprocessed: PreprocessedMessage,
}

/// Chat room client for Steam group chat functionality
pub struct ChatRoomClient {
    connection: steam_vent::Connection,
}

impl ChatRoomClient {
    /// Create a new chat room client from an existing connection
    pub fn new(connection: steam_vent::Connection) -> Self {
        Self { connection }
    }

    /// Get all chat room groups that the user is a member of
    pub async fn get_my_chat_rooms(&self) -> Result<Vec<ChatRoomInfo>, Box<dyn Error>> {
        let req = CChatRoom_GetMyChatRoomGroups_Request::new();
        let response: CChatRoom_GetMyChatRoomGroups_Response = self.connection.service_method(req).await?;

        let mut chat_rooms = Vec::new();
        for pair in &response.chat_room_groups {
            if let Some(group_summary) = pair.group_summary.as_ref() {
                let chat_room = ChatRoomInfo {
                    chat_group_id: group_summary.chat_group_id(),
                    chat_id: group_summary.default_chat_id(),
                    chat_name: group_summary.chat_group_name().to_string(),
                    chat_group_name: group_summary.chat_group_name().to_string(),
                    is_joined: true,
                };
                chat_rooms.push(chat_room);
            }
        }

        Ok(chat_rooms)
    }

    /// Join a chat room group
    pub async fn join_chat_room(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        invite_code: Option<String>,
    ) -> Result<CChatRoom_JoinChatRoomGroup_Response, Box<dyn Error>> {
        let mut req = CChatRoom_JoinChatRoomGroup_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);
        
        if let Some(code) = invite_code {
            req.set_invite_code(code);
        }

        let response: CChatRoom_JoinChatRoomGroup_Response = self.connection.service_method(req).await?;
        Ok(response)
    }

    /// Leave a chat room group
    pub async fn leave_chat_room(&self, chat_group_id: u64) -> Result<(), Box<dyn Error>> {
        let mut req = CChatRoom_LeaveChatRoomGroup_Request::new();
        req.set_chat_group_id(chat_group_id);

        let _response: CChatRoom_LeaveChatRoomGroup_Response = self.connection.service_method(req).await?;
        Ok(())
    }

    /// Send a message to a group chat with preprocessing
    pub async fn send_group_message(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        message: &str,
        echo_to_sender: bool,
    ) -> Result<PreprocessedMessage, Box<dyn Error>> {
        // Preprocess the message
        let preprocessed = MessagePreprocessor::preprocess_message(message);
        let prepared_message = MessagePreprocessor::prepare_message_for_sending(message);

        let mut req = CChatRoom_SendChatMessage_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);
        req.set_message(prepared_message);
        req.set_echo_to_sender(echo_to_sender);

        let response: CChatRoom_SendChatMessage_Response = self.connection.service_method(req).await?;
        
        // Process the response with preprocessing
        let final_preprocessed = MessagePreprocessor::process_response(
            message,
            response.modified_message(),
            response.server_timestamp(),
            response.ordinal(),
        );

        Ok(final_preprocessed)
    }

    /// Send a message to a friend
    pub async fn send_friend_message(
        &self,
        friend_steam_id: SteamID,
        message: &str,
        chat_entry_type: i32,
    ) -> Result<CFriendMessages_SendMessage_Response, Box<dyn Error>> {
        let mut req = CFriendMessages_SendMessage_Request::new();
        req.set_steamid(friend_steam_id.into());
        req.set_message(message.to_string());
        req.set_chat_entry_type(chat_entry_type);
        req.set_echo_to_sender(true);

        let response: CFriendMessages_SendMessage_Response = self.connection.service_method(req).await?;
        Ok(response)
    }

    /// Get chat room group state
    pub async fn get_chat_room_state(&self, chat_group_id: u64) -> Result<CChatRoom_GetChatRoomGroupState_Response, Box<dyn Error>> {
        let mut req = CChatRoom_GetChatRoomGroupState_Request::new();
        req.set_chat_group_id(chat_group_id);

        let response: CChatRoom_GetChatRoomGroupState_Response = self.connection.service_method(req).await?;
        Ok(response)
    }

    /// Listen for incoming group chat messages with preprocessing
    pub async fn listen_for_group_messages<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(EnhancedGroupChatMessage) + Send + 'static,
    {
        let mut incoming_messages = self.connection.on_notification::<CChatRoom_IncomingChatMessage_Notification>();

        while let Some(Ok(notification)) = incoming_messages.next().await {
            let preprocessed = MessagePreprocessor::preprocess_message(notification.message());
            
            let enhanced_message = EnhancedGroupChatMessage {
                chat_group_id: notification.chat_group_id(),
                chat_id: notification.chat_id(),
                sender_steam_id: SteamID::from(notification.steamid_sender()),
                message: notification.message().to_string(),
                timestamp: notification.timestamp(),
                chat_name: notification.chat_name().to_string(),
                ordinal: notification.ordinal(),
                preprocessed,
            };

            callback(enhanced_message);
        }

        Ok(())
    }

    /// Listen for incoming friend messages
    pub async fn listen_for_friend_messages<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessage) + Send + 'static,
    {
        let mut incoming_messages = self.connection.on_notification::<CFriendMessages_IncomingMessage_Notification>();

        while let Some(Ok(notification)) = incoming_messages.next().await {
            let friend_message = FriendMessage {
                steam_id: SteamID::from(notification.steamid_friend()),
                message: notification.message().to_string(),
                timestamp: notification.rtime32_server_timestamp(),
                chat_entry_type: notification.chat_entry_type(),
            };

            callback(friend_message);
        }

        Ok(())
    }

    /// Get the underlying connection for advanced operations
    pub fn connection(&self) -> &steam_vent::Connection {
        &self.connection
    }

    /// Get a mutable reference to the connection
    pub fn connection_mut(&mut self) -> &mut steam_vent::Connection {
        &mut self.connection
    }
}

/// Helper functions for chat operations
pub mod helpers {
    use super::*;

    /// Create a simple chat room client from a LogOn instance
    pub fn create_chat_client(logon: &crate::LogOn) -> ChatRoomClient {
        ChatRoomClient::new(logon.connection().clone())
    }

    /// Format a Steam ID for display
    pub fn format_steam_id(steam_id: SteamID) -> String {
        steam_id.steam3().to_string()
    }

    /// Parse a Steam ID from string
    pub fn parse_steam_id(steam_id_str: &str) -> Result<SteamID, Box<dyn Error>> {
        Ok(SteamID::try_from(steam_id_str)?)
    }

    /// Create a message with mentions
    pub fn create_message_with_mentions(message: &str, steam_ids: &[SteamID]) -> String {
        let mut result = message.to_string();
        for steam_id in steam_ids {
            result.push_str(&format!(" @{}", steam_id.steam3()));
        }
        result
    }

    /// Create a message with @all mention
    pub fn create_message_with_all_mention(message: &str) -> String {
        format!("@all {}", message)
    }

    /// Create a message with @here mention
    pub fn create_message_with_here_mention(message: &str) -> String {
        format!("@here {}", message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogOn;

    #[tokio::test]
    async fn test_chat_client_creation() {
        // Test with anonymous connection
        let logon = LogOn::new_anonymous().await.unwrap();
        let chat_client = ChatRoomClient::new(logon.connection().clone());
        
        // Test getting chat rooms (should work even if empty)
        let chat_rooms = chat_client.get_my_chat_rooms().await;
        assert!(chat_rooms.is_ok(), "Should be able to get chat rooms");
        
        let rooms = chat_rooms.unwrap();
        println!("Found {} chat rooms", rooms.len());
    }

    #[tokio::test]
    async fn test_steam_id_parsing() {
        let steam_id_str = "[U:1:1531059355]";
        let steam_id = helpers::parse_steam_id(steam_id_str);
        assert!(steam_id.is_ok(), "Should parse valid Steam ID");
        
        let formatted = helpers::format_steam_id(steam_id.unwrap());
        assert_eq!(formatted, steam_id_str);
    }

    #[test]
    fn test_message_with_mentions() {
        let steam_id = SteamID::try_from("[U:1:1531059355]").unwrap();
        let message = helpers::create_message_with_mentions("Hello", &[steam_id]);
        assert!(message.contains("@"));
    }
} 