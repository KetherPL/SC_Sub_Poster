// SPDX-License-Identifier: LGPL-3.0-only

use std::error::Error;
use futures_util::StreamExt as FuturesStreamExt;
use std::pin::Pin;
use std::time::Duration;
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
use thiserror::Error;
use tokio::time::sleep;
use tokio_stream::{Stream, StreamExt};
use crate::preprocessing::{MessagePreprocessor, PreprocessedMessage};
use tracing::{debug, instrument};

type CallbackResult = Result<(), Box<dyn Error + Send + Sync>>;

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

impl EnhancedGroupChatMessage {
    /// Create an enhanced message from a notification, preserving the whole notification object
    pub fn from_notification(
        notification: &CChatRoom_IncomingChatMessage_Notification,
    ) -> Self {
        let preprocessed = MessagePreprocessor::preprocess_message(notification.message());
        Self {
            chat_group_id: notification.chat_group_id(),
            chat_id: notification.chat_id(),
            sender_steam_id: SteamID::from(notification.steamid_sender() as u64),
            message: notification.message().to_string(),
            timestamp: notification.timestamp(),
            chat_name: notification.chat_name().to_string(),
            ordinal: notification.ordinal(),
            preprocessed,
        }
    }
}

/// Chat room client for Steam group chat functionality
pub struct ChatRoomClient {
    connection: steam_vent::Connection,
}

/// Group-related operations for chat rooms.
pub struct ChatRoomGroups<'a> {
    connection: &'a steam_vent::Connection,
}

/// Message sending helpers for chats and friends.
pub struct ChatRoomMessaging<'a> {
    connection: &'a steam_vent::Connection,
}

/// Notification listeners for chat and friend messages.
pub struct ChatRoomNotifications<'a> {
    connection: &'a steam_vent::Connection,
}

/// Parameters for sending a group message
#[derive(Debug, Clone)]
pub struct SendGroupMessageParams {
    pub chat_group_id: u64,
    pub chat_id: u64,
    pub message: String,
    pub echo_to_sender: bool,
}

impl SendGroupMessageParams {
    pub fn new(chat_group_id: u64, chat_id: u64, message: impl Into<String>) -> Self {
        Self {
            chat_group_id,
            chat_id,
            message: message.into(),
            echo_to_sender: true,
        }
    }

    pub fn with_echo_to_sender(mut self, echo: bool) -> Self {
        self.echo_to_sender = echo;
        self
    }
}

struct NotificationStream<'a, T> {
    inner: Pin<Box<dyn Stream<Item = Result<T, steam_vent::NetworkError>> + Send + 'a>>,
    backoff: Duration,
}

#[derive(Debug, Error)]
enum NotificationDispatchError {
    #[error("notification stream error: {0}")]
    Stream(#[from] steam_vent::NetworkError),
    #[error("notification callback failed")]
    Callback {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl ChatRoomClient {
    /// Create a new chat room client from an existing connection
    pub fn new(connection: steam_vent::Connection) -> Self {
        Self { connection }
    }

    /// Access group-related operations.
    pub fn groups(&self) -> ChatRoomGroups<'_> {
        ChatRoomGroups {
            connection: &self.connection,
        }
    }

    /// Access message sending helpers.
    pub fn messaging(&self) -> ChatRoomMessaging<'_> {
        ChatRoomMessaging {
            connection: &self.connection,
        }
    }

    /// Access notification listeners.
    pub fn notifications(&self) -> ChatRoomNotifications<'_> {
        ChatRoomNotifications {
            connection: &self.connection,
        }
    }

    /// Get all chat room groups that the user is a member of
    pub async fn get_my_chat_rooms(&self) -> Result<Vec<ChatRoomInfo>, Box<dyn Error>> {
        self.groups().get_my_chat_rooms().await
    }

    /// Join a chat room group
    pub async fn join_chat_room(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        invite_code: Option<String>,
    ) -> Result<CChatRoom_JoinChatRoomGroup_Response, Box<dyn Error>> {
        self.groups()
            .join_chat_room(chat_group_id, chat_id, invite_code)
            .await
    }

    /// Leave a chat room group
    pub async fn leave_chat_room(&self, chat_group_id: u64) -> Result<(), Box<dyn Error>> {
        self.groups().leave_chat_room(chat_group_id).await
    }

    /// Send a message to a group chat with preprocessing
    #[instrument(
        name = "kether.chat.send_group_message",
        skip(self, params),
        fields(chat_group_id = params.chat_group_id, chat_id = params.chat_id)
    )]
    pub async fn send_group_message(
        &self,
        params: SendGroupMessageParams,
    ) -> Result<PreprocessedMessage, Box<dyn Error>> {
        self.messaging().send_group_message(params).await
    }

    /// Send a message to a friend
    pub async fn send_friend_message(
        &self,
        friend_steam_id: SteamID,
        message: &str,
        chat_entry_type: i32,
    ) -> Result<CFriendMessages_SendMessage_Response, Box<dyn Error>> {
        self.messaging()
            .send_friend_message(friend_steam_id, message, chat_entry_type)
            .await
    }

    /// Get chat room group state
    pub async fn get_chat_room_state(&self, chat_group_id: u64) -> Result<CChatRoom_GetChatRoomGroupState_Response, Box<dyn Error>> {
        self.groups().get_chat_room_state(chat_group_id).await
    }

    /// Listen for incoming group chat messages with preprocessing
    pub async fn listen_for_group_messages<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(EnhancedGroupChatMessage) + Send + 'static,
    {
        self.notifications().listen_for_group_messages(callback).await
    }

    /// Listen for incoming friend messages
    pub async fn listen_for_friend_messages<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessage) + Send + 'static,
    {
        self.notifications().listen_for_friend_messages(callback).await
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

impl<'a, T> NotificationStream<'a, T>
where
    T: Send + 'static,
{
    fn new<S>(stream: S, backoff: Duration) -> Self
    where
        S: Stream<Item = Result<T, steam_vent::NetworkError>> + Send + 'a,
    {
        Self {
            inner: FuturesStreamExt::boxed(stream),
            backoff,
        }
    }

    async fn for_each<F>(mut self, mut handler: F) -> Result<(), NotificationDispatchError>
    where
        F: FnMut(T) -> CallbackResult + Send + 'static,
    {
        while let Some(result) = StreamExt::next(&mut self.inner).await {
            match result {
                Ok(item) => handler(item).map_err(|source| NotificationDispatchError::Callback { source })?,
                Err(err) => {
                    sleep(self.backoff).await;
                    return Err(NotificationDispatchError::Stream(err));
                }
            }
        }

        Ok(())
    }
}

impl<'a> ChatRoomGroups<'a> {
    pub async fn get_my_chat_rooms(&self) -> Result<Vec<ChatRoomInfo>, Box<dyn Error>> {
        let req = CChatRoom_GetMyChatRoomGroups_Request::new();
        let response: CChatRoom_GetMyChatRoomGroups_Response =
            self.connection.service_method(req).await?;

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

        let response: CChatRoom_JoinChatRoomGroup_Response =
            self.connection.service_method(req).await?;
        Ok(response)
    }

    pub async fn leave_chat_room(&self, chat_group_id: u64) -> Result<(), Box<dyn Error>> {
        let mut req = CChatRoom_LeaveChatRoomGroup_Request::new();
        req.set_chat_group_id(chat_group_id);

        let _response: CChatRoom_LeaveChatRoomGroup_Response =
            self.connection.service_method(req).await?;
        Ok(())
    }

    pub async fn get_chat_room_state(
        &self,
        chat_group_id: u64,
    ) -> Result<CChatRoom_GetChatRoomGroupState_Response, Box<dyn Error>> {
        let mut req = CChatRoom_GetChatRoomGroupState_Request::new();
        req.set_chat_group_id(chat_group_id);

        let response: CChatRoom_GetChatRoomGroupState_Response =
            self.connection.service_method(req).await?;
        Ok(response)
    }
}

impl<'a> ChatRoomMessaging<'a> {
    pub async fn send_group_message(
        &self,
        params: SendGroupMessageParams,
    ) -> Result<PreprocessedMessage, Box<dyn Error>> {
        let req = Self::build_send_message_request(&params);
        let response: CChatRoom_SendChatMessage_Response =
            self.connection.service_method(req).await?;
        let final_preprocessed = Self::process_send_message_response(&params, &response);

        debug!(
            chat_group_id = params.chat_group_id,
            chat_id = params.chat_id,
            ordinal = response.ordinal(),
            "group message dispatched"
        );

        Ok(final_preprocessed)
    }

    fn build_send_message_request(params: &SendGroupMessageParams) -> CChatRoom_SendChatMessage_Request {
        let prepared_message = MessagePreprocessor::prepare_message_for_sending(&params.message);
        let mut req = CChatRoom_SendChatMessage_Request::new();
        req.set_chat_group_id(params.chat_group_id);
        req.set_chat_id(params.chat_id);
        req.set_message(prepared_message);
        req.set_echo_to_sender(params.echo_to_sender);
        req
    }

    fn process_send_message_response(
        params: &SendGroupMessageParams,
        response: &CChatRoom_SendChatMessage_Response,
    ) -> PreprocessedMessage {
        MessagePreprocessor::process_response(
            &params.message,
            response.modified_message(),
            response.server_timestamp(),
            response.ordinal(),
        )
    }

    #[instrument(
        name = "kether.chat.send_friend_message",
        skip(self, message),
        fields(friend = %friend_steam_id.steam3(), chat_entry_type)
    )]
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

        let response: CFriendMessages_SendMessage_Response =
            self.connection.service_method(req).await?;

        debug!(
            friend = %friend_steam_id.steam3(),
            chat_entry_type,
            "friend message dispatched"
        );
        Ok(response)
    }
}

impl<'a> ChatRoomNotifications<'a> {
    pub async fn listen_for_group_messages_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(EnhancedGroupChatMessage) -> CallbackResult + Send + 'static,
    {
        let mut user_callback = callback;
        self.group_stream()
            .for_each(move |notification| {
                let enhanced_message = EnhancedGroupChatMessage::from_notification(&notification);
                user_callback(enhanced_message)
            })
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(err) })
    }

    pub async fn listen_for_group_messages<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(EnhancedGroupChatMessage) + Send + 'static,
    {
        self.listen_for_group_messages_with(move |message| {
            callback(message);
            Ok(())
            })
        .await
    }

    pub async fn listen_for_friend_messages_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessage) -> CallbackResult + Send + 'static,
    {
        let mut user_callback = callback;
        self.friend_stream()
            .for_each(move |notification| {
                let friend_message = FriendMessage {
                    steam_id: SteamID::from(notification.steamid_friend()),
                    message: notification.message().to_string(),
                    timestamp: notification.rtime32_server_timestamp(),
                    chat_entry_type: notification.chat_entry_type(),
                };
                user_callback(friend_message)
            })
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(err) })
    }

    pub async fn listen_for_friend_messages<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessage) + Send + 'static,
    {
        self.listen_for_friend_messages_with(move |message| {
            callback(message);
            Ok(())
        })
        .await
    }

    fn group_stream(
        &self,
    ) -> NotificationStream<'_, CChatRoom_IncomingChatMessage_Notification> {
        let stream = self
            .connection
            .on_notification::<CChatRoom_IncomingChatMessage_Notification>()
            .throttle(Duration::from_millis(25));
        NotificationStream::new(stream, Duration::from_millis(250))
    }

    fn friend_stream(
        &self,
    ) -> NotificationStream<'_, CFriendMessages_IncomingMessage_Notification> {
        let stream = self
            .connection
            .on_notification::<CFriendMessages_IncomingMessage_Notification>()
            .throttle(Duration::from_millis(25));
        NotificationStream::new(stream, Duration::from_millis(250))
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
        format!("{} {}", crate::preprocessing::MENTION_ALL, message)
    }

    /// Create a message with @here mention
    pub fn create_message_with_here_mention(message: &str) -> String {
        format!("{} {}", crate::preprocessing::MENTION_HERE, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogOn;

    #[tokio::test]
    #[ignore = "Requires Steam network access"]
    async fn test_chat_client_creation() {
        // Try to get credentials from environment variables
        let logon = match (std::env::var("STEAM_ACCOUNT"), std::env::var("STEAM_PASSWORD")) {
            (Ok(account), Ok(password)) => {
                println!("Using credentials from environment variables");
                LogOn::new(&account, &password).await
            }
            _ => {
                println!("No credentials provided, using anonymous connection");
                LogOn::new_anonymous().await
            }
        }.unwrap();
        
        let chat_client = ChatRoomClient::new(logon.connection().clone());
        
        // Test getting chat rooms (should work even if empty)
        let chat_rooms = chat_client.get_my_chat_rooms().await;
        assert!(chat_rooms.is_ok(), "Should be able to get chat rooms");
        
        let rooms = chat_rooms.unwrap();
        println!("Found {} chat rooms:", rooms.len());
        
        for (i, room) in rooms.iter().enumerate() {
            println!("  {}. {} (Group: {})", i + 1, room.chat_name, room.chat_group_name);
            println!("     Group ID: {}, Chat ID: {}", room.chat_group_id, room.chat_id);
        }
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