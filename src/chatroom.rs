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
    CChatRoom_DeleteChatMessages_Request,
    CChatRoom_DeleteChatMessages_Response,
    cchat_room_delete_chat_messages_request,
};
use steam_vent::proto::steammessages_friendmessages_steamclient::{
    CFriendMessages_SendMessage_Request,
    CFriendMessages_SendMessage_Response,
    CFriendMessages_IncomingMessage_Notification,
};
use steam_vent::ConnectionTrait;
use steamid_ng::SteamID;
use thiserror::Error;
use tokio::time::{sleep, timeout, Duration as TokioDuration};
use tokio_stream::{Stream, StreamExt};
use crate::preprocessing::{MessagePreprocessor, PreprocessedMessage};
use tracing::{debug, instrument};

type CallbackResult = Result<(), Box<dyn Error + Send + Sync>>;

/// Chat room information
#[derive(Debug, Clone)]
pub struct ChatRoomInfo {
    /// The unique identifier for the chat group.
    pub chat_group_id: u64,
    /// The unique identifier for the specific chat room within the group.
    pub chat_id: u64,
    /// The display name of the chat room.
    pub chat_name: String,
    /// The display name of the chat group.
    pub chat_group_name: String,
    /// Whether the current user is currently joined to this chat room.
    pub is_joined: bool,
}

/// Friend message information
#[derive(Debug, Clone)]
pub struct FriendMessage {
    /// The Steam ID of the friend who sent the message.
    pub steam_id: SteamID,
    /// The message text content.
    pub message: String,
    /// Unix timestamp when the message was sent.
    pub timestamp: u32,
    /// The type of chat entry (message type identifier from Steam API).
    pub chat_entry_type: i32,
}

/// Group chat message information
#[derive(Debug, Clone)]
pub struct GroupChatMessage {
    /// The unique identifier for the chat group.
    pub chat_group_id: u64,
    /// The unique identifier for the specific chat room within the group.
    pub chat_id: u64,
    /// The Steam ID of the user who sent the message.
    pub sender_steam_id: SteamID,
    /// The message text content.
    pub message: String,
    /// Unix timestamp when the message was sent.
    pub timestamp: u32,
    /// The display name of the chat room.
    pub chat_name: String,
    /// Message ordinal/sequence number assigned by the server.
    pub ordinal: u32,
}

/// Enhanced group chat message with preprocessing
///
/// Extends `GroupChatMessage` with preprocessed BBCode and mention information.
#[derive(Debug, Clone)]
pub struct EnhancedGroupChatMessage {
    /// The unique identifier for the chat group.
    pub chat_group_id: u64,
    /// The unique identifier for the specific chat room within the group.
    pub chat_id: u64,
    /// The Steam ID of the user who sent the message.
    pub sender_steam_id: SteamID,
    /// The message text content.
    pub message: String,
    /// Unix timestamp when the message was sent.
    pub timestamp: u32,
    /// The display name of the chat room.
    pub chat_name: String,
    /// Message ordinal/sequence number assigned by the server.
    pub ordinal: u32,
    /// Preprocessed message data including parsed BBCode and extracted mentions.
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
    /// The unique identifier for the chat group.
    pub chat_group_id: u64,
    /// The unique identifier for the specific chat room within the group.
    pub chat_id: u64,
    /// The message text to send.
    pub message: String,
    /// Whether the message should be echoed back to the sender.
    pub echo_to_sender: bool,
}

impl SendGroupMessageParams {
    /// Create a new `SendGroupMessageParams` with default settings.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room
    /// * `message` - The message text to send (any type that can be converted to `String`)
    ///
    /// # Defaults
    ///
    /// * `echo_to_sender` is set to `false` by default. Use `with_echo_to_sender()` to change it.
    pub fn new(chat_group_id: u64, chat_id: u64, message: impl Into<String>) -> Self {
        Self {
            chat_group_id,
            chat_id,
            message: message.into(),
            echo_to_sender: false,
        }
    }

    /// Set whether the message should be echoed back to the sender.
    ///
    /// # Arguments
    ///
    /// * `echo` - If `true`, the message will be echoed back to the sender
    ///
    /// # Returns
    ///
    /// `Self` for method chaining (builder pattern).
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
    /// Create a new chat room client from an existing connection.
    ///
    /// # Arguments
    ///
    /// * `connection` - An established Steam connection from `LogOn::connection()`
    pub fn new(connection: steam_vent::Connection) -> Self {
        Self { connection }
    }

    /// Access group-related operations (joining, leaving, listing chat rooms).
    ///
    /// # Returns
    ///
    /// A `ChatRoomGroups` handle for performing group operations.
    pub fn groups(&self) -> ChatRoomGroups<'_> {
        ChatRoomGroups {
            connection: &self.connection,
        }
    }

    /// Access message sending helpers for group chats and friend messages.
    ///
    /// # Returns
    ///
    /// A `ChatRoomMessaging` handle for sending messages.
    pub fn messaging(&self) -> ChatRoomMessaging<'_> {
        ChatRoomMessaging {
            connection: &self.connection,
        }
    }

    /// Access notification listeners for incoming messages.
    ///
    /// # Returns
    ///
    /// A `ChatRoomNotifications` handle for setting up message listeners.
    pub fn notifications(&self) -> ChatRoomNotifications<'_> {
        ChatRoomNotifications {
            connection: &self.connection,
        }
    }

    /// Get all chat room groups that the user is a member of.
    ///
    /// # Returns
    ///
    /// A list of `ChatRoomInfo` structures describing each chat room group the user belongs to.
    ///
    /// # Errors
    ///
    /// Returns an error if the Steam API request fails.
    pub async fn get_my_chat_rooms(&self) -> Result<Vec<ChatRoomInfo>, Box<dyn Error>> {
        self.groups().get_my_chat_rooms().await
    }

    /// Join a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room
    /// * `invite_code` - Optional invite code required for private chat rooms
    ///
    /// # Returns
    ///
    /// The Steam API response containing join confirmation details.
    ///
    /// # Errors
    ///
    /// Returns an error if the join request fails or if an invalid invite code is provided.
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

    /// Leave a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group to leave
    ///
    /// # Errors
    ///
    /// Returns an error if the leave request fails.
    pub async fn leave_chat_room(&self, chat_group_id: u64) -> Result<(), Box<dyn Error>> {
        self.groups().leave_chat_room(chat_group_id).await
    }

    /// Send a message to a group chat with preprocessing.
    ///
    /// The message will be preprocessed to extract BBCode and mentions before sending.
    ///
    /// # Arguments
    ///
    /// * `params` - Parameters for sending the message (see `SendGroupMessageParams`)
    ///
    /// # Returns
    ///
    /// A `PreprocessedMessage` containing the original message, server-modified version,
    /// parsed BBCode, and extracted mentions.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sending fails.
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

    /// Send a message to a friend.
    ///
    /// # Arguments
    ///
    /// * `friend_steam_id` - The Steam ID of the friend to send the message to
    /// * `message` - The message text to send
    /// * `chat_entry_type` - The type of chat entry (message type identifier from Steam API)
    ///
    /// # Returns
    ///
    /// The Steam API response containing message confirmation details.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sending fails.
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

    /// Delete one or more group chat messages.
    ///
    /// Messages are identified by their `server_timestamp` and `ordinal` values,
    /// which are returned when sending messages via `send_group_message()`.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room within the group
    /// * `messages` - A vector of (server_timestamp, ordinal) tuples identifying messages to delete
    ///
    /// # Returns
    ///
    /// The Steam API response confirming the deletion.
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion request fails. Note that attempting to delete
    /// messages that have already been deleted or don't exist may not result in an error.
    #[instrument(
        name = "kether.chat.delete_group_messages",
        skip(self, messages),
        fields(chat_group_id, chat_id, message_count = messages.len())
    )]
    pub async fn delete_group_messages(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        messages: Vec<(u32, u32)>,
    ) -> Result<CChatRoom_DeleteChatMessages_Response, Box<dyn Error>> {
        self.messaging()
            .delete_group_messages(chat_group_id, chat_id, messages)
            .await
    }

    /// Delete group chat messages from `PreprocessedMessage` objects.
    ///
    /// This is a convenience method that extracts message identifiers from
    /// `PreprocessedMessage` objects (returned by `send_group_message()`) and deletes them.
    /// Any messages where `server_timestamp` or `ordinal` is `None` will be skipped with a warning.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room within the group
    /// * `messages` - A vector of `PreprocessedMessage` objects to delete
    ///
    /// # Returns
    ///
    /// The Steam API response confirming the deletion, or an error if no valid messages
    /// were found or the deletion request fails.
    ///
    /// # Errors
    ///
    /// Returns an error if no valid message identifiers are found (all messages had
    /// missing `server_timestamp` or `ordinal`), if the messages list is empty, or if
    /// the deletion request fails.
    #[instrument(
        name = "kether.chat.delete_group_messages_from_preprocessed",
        skip(self, messages),
        fields(chat_group_id, chat_id, input_message_count = messages.len())
    )]
    pub async fn delete_group_messages_from_preprocessed(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        messages: Vec<PreprocessedMessage>,
    ) -> Result<CChatRoom_DeleteChatMessages_Response, Box<dyn Error>> {
        self.messaging()
            .delete_group_messages_from_preprocessed(chat_group_id, chat_id, messages)
            .await
    }

    /// Get the current state of a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    ///
    /// # Returns
    ///
    /// The Steam API response containing the current state of the chat room group.
    ///
    /// # Errors
    ///
    /// Returns an error if the state request fails.
    pub async fn get_chat_room_state(&self, chat_group_id: u64) -> Result<CChatRoom_GetChatRoomGroupState_Response, Box<dyn Error>> {
        self.groups().get_chat_room_state(chat_group_id).await
    }

    /// Listen for incoming group chat messages with preprocessing.
    ///
    /// Messages are automatically preprocessed to extract BBCode and mentions.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that will be called for each incoming message
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback panics.
    pub async fn listen_for_group_messages<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(EnhancedGroupChatMessage) + Send + 'static,
    {
        self.notifications().listen_for_group_messages(callback).await
    }

    /// Listen for incoming friend messages.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that will be called for each incoming friend message
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback panics.
    pub async fn listen_for_friend_messages<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessage) + Send + 'static,
    {
        self.notifications().listen_for_friend_messages(callback).await
    }

    /// Get the underlying Steam connection for advanced operations.
    ///
    /// This provides direct access to the `steam-vent` connection, allowing
    /// you to perform operations not covered by the high-level API.
    pub fn connection(&self) -> &steam_vent::Connection {
        &self.connection
    }

    /// Get a mutable reference to the underlying Steam connection.
    ///
    /// This provides mutable access to the `steam-vent` connection for advanced use cases.
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
    /// Get all chat room groups that the user is a member of.
    ///
    /// # Returns
    ///
    /// A list of `ChatRoomInfo` structures describing each chat room group the user belongs to.
    ///
    /// # Errors
    ///
    /// Returns an error if the Steam API request fails.
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

    /// Join a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room
    /// * `invite_code` - Optional invite code required for private chat rooms
    ///
    /// # Returns
    ///
    /// The Steam API response containing join confirmation details.
    ///
    /// # Errors
    ///
    /// Returns an error if the join request fails or if an invalid invite code is provided.
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

    /// Leave a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group to leave
    ///
    /// # Errors
    ///
    /// Returns an error if the leave request fails.
    pub async fn leave_chat_room(&self, chat_group_id: u64) -> Result<(), Box<dyn Error>> {
        let mut req = CChatRoom_LeaveChatRoomGroup_Request::new();
        req.set_chat_group_id(chat_group_id);

        let _response: CChatRoom_LeaveChatRoomGroup_Response =
            self.connection.service_method(req).await?;
        Ok(())
    }

    /// Get the current state of a chat room group.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    ///
    /// # Returns
    ///
    /// The Steam API response containing the current state of the chat room group.
    ///
    /// # Errors
    ///
    /// Returns an error if the state request fails.
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
    /// Send a message to a group chat with preprocessing.
    ///
    /// The message will be preprocessed to extract BBCode and mentions before sending.
    ///
    /// # Arguments
    ///
    /// * `params` - Parameters for sending the message (see `SendGroupMessageParams`)
    ///
    /// # Returns
    ///
    /// A `PreprocessedMessage` containing the original message, server-modified version,
    /// parsed BBCode, and extracted mentions.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sending fails.
    pub async fn send_group_message(
        &self,
        params: SendGroupMessageParams,
    ) -> Result<PreprocessedMessage, Box<dyn Error>> {
        let req = Self::build_send_message_request(&params);
        let response: CChatRoom_SendChatMessage_Response =
            self.connection.service_method(req).await?;
        let mut final_preprocessed = Self::process_send_message_response(&params, &response);

        // If echo_to_sender is true and ordinal is not available, wait for the echo notification
        // to get the correct ordinal (which is needed for message deletion)
        if params.echo_to_sender && final_preprocessed.ordinal.is_none() {
            if let Ok(notification) = Self::wait_for_echo_notification(
                self.connection,
                params.chat_group_id,
                params.chat_id,
                TokioDuration::from_secs(5),
            ).await {
                // Update the preprocessed message with ordinal and timestamp from notification
                final_preprocessed.ordinal = Some(notification.ordinal());
                final_preprocessed.server_timestamp = Some(notification.timestamp());
            } else {
                tracing::warn!(
                    chat_group_id = params.chat_group_id,
                    chat_id = params.chat_id,
                    "Timeout waiting for echo notification; ordinal not available for deletion"
                );
            }
        }

        debug!(
            chat_group_id = params.chat_group_id,
            chat_id = params.chat_id,
            ordinal = final_preprocessed.ordinal.unwrap_or(0),
            "group message dispatched"
        );

        Ok(final_preprocessed)
    }

    /// Wait for an echo notification matching the given chat_group_id and chat_id.
    ///
    /// This is used to get the ordinal from the echo notification when sending a message.
    async fn wait_for_echo_notification(
        connection: &steam_vent::Connection,
        chat_group_id: u64,
        chat_id: u64,
        timeout_duration: TokioDuration,
    ) -> Result<CChatRoom_IncomingChatMessage_Notification, Box<dyn Error>> {
        let stream = connection
            .on_notification::<CChatRoom_IncomingChatMessage_Notification>()
            .throttle(Duration::from_millis(25));
        
        let mut pinned_stream: Pin<Box<dyn Stream<Item = Result<CChatRoom_IncomingChatMessage_Notification, steam_vent::NetworkError>> + Send>> = 
            Box::pin(stream);

        let result = timeout(timeout_duration, async move {
            loop {
                if let Some(Ok(notification)) = StreamExt::next(&mut pinned_stream).await {
                    // Match notification to our sent message by chat_group_id and chat_id
                    if notification.chat_group_id() == chat_group_id
                        && notification.chat_id() == chat_id
                    {
                        return Ok(notification);
                    }
                } else {
                    return Err("Notification stream ended".into());
                }
            }
        }).await;

        match result {
            Ok(Ok(notification)) => Ok(notification),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Timeout waiting for echo notification".into()),
        }
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
        // Steam may not populate ordinal in the response (or set it to 0).
        // If echo_to_sender is true, the ordinal will be available in the notification.
        // process_response will set ordinal to None if it's 0, indicating it needs to be
        // obtained from the notification.
        let ordinal = if response.has_ordinal() {
            response.ordinal()
        } else {
            // Not set in response - will be available in notification if echo_to_sender is true
            0
        };
        
        MessagePreprocessor::process_response(
            &params.message,
            response.modified_message(),
            response.server_timestamp(),
            ordinal,
        )
    }

    /// Update a PreprocessedMessage with ordinal and timestamp from a notification.
    ///
    /// This is useful when you need the ordinal from the echo notification to delete a message.
    ///
    /// # Arguments
    ///
    /// * `preprocessed` - The PreprocessedMessage to update
    /// * `notification` - The incoming chat message notification containing the ordinal
    ///
    /// # Returns
    ///
    /// A new PreprocessedMessage with updated ordinal and server_timestamp from the notification.
    pub fn update_preprocessed_from_notification(
        preprocessed: &PreprocessedMessage,
        notification: &CChatRoom_IncomingChatMessage_Notification,
    ) -> PreprocessedMessage {
        PreprocessedMessage {
            original_message: preprocessed.original_message.clone(),
            modified_message: notification.message().to_string(),
            message_bbcode_parsed: MessagePreprocessor::parse_bbcode(notification.message()),
            mentions: MessagePreprocessor::extract_mentions(notification.message()),
            server_timestamp: Some(notification.timestamp()),
            ordinal: Some(notification.ordinal()),
        }
    }

    /// Send a message to a friend.
    ///
    /// # Arguments
    ///
    /// * `friend_steam_id` - The Steam ID of the friend to send the message to
    /// * `message` - The message text to send
    /// * `chat_entry_type` - The type of chat entry (message type identifier from Steam API)
    ///
    /// # Returns
    ///
    /// The Steam API response containing message confirmation details.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sending fails.
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

    /// Delete one or more group chat messages.
    ///
    /// Messages are identified by their `server_timestamp` and `ordinal` values,
    /// which are returned when sending messages via `send_group_message()`.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room within the group
    /// * `messages` - A vector of (server_timestamp, ordinal) tuples identifying messages to delete
    ///
    /// # Returns
    ///
    /// The Steam API response confirming the deletion.
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion request fails. Note that attempting to delete
    /// messages that have already been deleted or don't exist may not result in an error.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // After sending a message and receiving a PreprocessedMessage:
    /// let preprocessed = client.send_group_message(params).await?;
    /// if let (Some(ts), Some(ord)) = (preprocessed.server_timestamp, preprocessed.ordinal) {
    ///     client.messaging().delete_group_messages(
    ///         chat_group_id,
    ///         chat_id,
    ///         vec![(ts, ord)]
    ///     ).await?;
    /// }
    /// ```
    #[instrument(
        name = "kether.chat.delete_group_messages",
        skip(self, messages),
        fields(chat_group_id, chat_id, message_count = messages.len())
    )]
    pub async fn delete_group_messages(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        messages: Vec<(u32, u32)>,
    ) -> Result<CChatRoom_DeleteChatMessages_Response, Box<dyn Error>> {
        if messages.is_empty() {
            return Err("Cannot delete empty list of messages".into());
        }

        let message_count = messages.len();
        let mut req = CChatRoom_DeleteChatMessages_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);

        // Convert (server_timestamp, ordinal) tuples to Message structs
        for (server_timestamp, ordinal) in messages {
            let mut msg = cchat_room_delete_chat_messages_request::Message::new();
            msg.set_server_timestamp(server_timestamp);
            msg.set_ordinal(ordinal);
            req.messages.push(msg);
        }

        let response: CChatRoom_DeleteChatMessages_Response =
            self.connection.service_method(req).await?;

        debug!(
            chat_group_id,
            chat_id,
            message_count,
            "group messages deleted"
        );

        Ok(response)
    }

    /// Delete group chat messages from `PreprocessedMessage` objects.
    ///
    /// This is a convenience method that extracts message identifiers from
    /// `PreprocessedMessage` objects (returned by `send_group_message()`) and deletes them.
    /// Any messages where `server_timestamp` or `ordinal` is `None` will be skipped with a warning.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room within the group
    /// * `messages` - A vector of `PreprocessedMessage` objects to delete
    ///
    /// # Returns
    ///
    /// The Steam API response confirming the deletion, or an error if no valid messages
    /// were found or the deletion request fails.
    ///
    /// # Errors
    ///
    /// Returns an error if no valid message identifiers are found (all messages had
    /// missing `server_timestamp` or `ordinal`), if the messages list is empty, or if
    /// the deletion request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // Send a message and store it
    /// let sent_message = client.send_group_message(params).await?;
    ///
    /// // Later, delete it
    /// client.messaging().delete_group_messages_from_preprocessed(
    ///     chat_group_id,
    ///     chat_id,
    ///     vec![sent_message]
    /// ).await?;
    /// ```
    #[instrument(
        name = "kether.chat.delete_group_messages_from_preprocessed",
        skip(self, messages),
        fields(chat_group_id, chat_id, input_message_count = messages.len())
    )]
    pub async fn delete_group_messages_from_preprocessed(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        messages: Vec<PreprocessedMessage>,
    ) -> Result<CChatRoom_DeleteChatMessages_Response, Box<dyn Error>> {
        // Extract valid (server_timestamp, ordinal) pairs, filtering out None values
        let mut message_identifiers = Vec::new();
        let mut skipped_count = 0;

        for msg in &messages {
            match (msg.server_timestamp, msg.ordinal) {
                (Some(ts), Some(ord)) => {
                    message_identifiers.push((ts, ord));
                }
                _ => {
                    skipped_count += 1;
                    tracing::warn!(
                        "Skipping message deletion: missing server_timestamp or ordinal"
                    );
                }
            }
        }

        if message_identifiers.is_empty() {
            if skipped_count > 0 {
                return Err(format!(
                    "All {} message(s) had missing server_timestamp or ordinal",
                    messages.len()
                )
                .into());
            } else {
                return Err("Cannot delete empty list of messages".into());
            }
        }

        if skipped_count > 0 {
            tracing::warn!(
                skipped_count,
                total = messages.len(),
                "Skipped messages with missing identifiers"
            );
        }

        self.delete_group_messages(chat_group_id, chat_id, message_identifiers)
            .await
    }
}

impl<'a> ChatRoomNotifications<'a> {
    /// Listen for incoming group chat messages with preprocessing and error handling.
    ///
    /// Messages are automatically preprocessed to extract BBCode and mentions.
    /// The callback can return an error to stop the listener, or `Ok(())` to continue.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that processes each incoming message and returns a `CallbackResult`
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
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

    /// Listen for incoming group chat messages with preprocessing.
    ///
    /// Messages are automatically preprocessed to extract BBCode and mentions.
    /// This is a convenience wrapper that ignores callback errors.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that will be called for each incoming message
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails.
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

    /// Listen for incoming friend messages with error handling.
    ///
    /// The callback can return an error to stop the listener, or `Ok(())` to continue.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that processes each incoming message and returns a `CallbackResult`
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
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

    /// Listen for incoming friend messages.
    ///
    /// This is a convenience wrapper that ignores callback errors.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that will be called for each incoming friend message
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails.
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