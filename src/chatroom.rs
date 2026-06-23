// SPDX-License-Identifier: LGPL-3.0-only

use crate::preprocessing::{MessagePreprocessor, PreprocessedMessage};
use futures_util::StreamExt as FuturesStreamExt;
use std::error::Error;
use std::pin::Pin;
use std::time::Duration;
use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_DeleteChatMessages_Request, CChatRoom_DeleteChatMessages_Response,
    CChatRoom_GetChatRoomGroupState_Request, CChatRoom_GetChatRoomGroupState_Response,
    CChatRoom_GetMessageHistory_Request, CChatRoom_GetMessageHistory_Response,
    CChatRoom_GetMessageReactionReactors_Request, CChatRoom_GetMessageReactionReactors_Response,
    CChatRoom_GetMyChatRoomGroups_Request, CChatRoom_GetMyChatRoomGroups_Response,
    CChatRoom_IncomingChatMessage_Notification, CChatRoom_JoinChatRoomGroup_Request,
    CChatRoom_JoinChatRoomGroup_Response, CChatRoom_LeaveChatRoomGroup_Request,
    CChatRoom_LeaveChatRoomGroup_Response, CChatRoom_MessageReaction_Notification,
    CChatRoom_SendChatMessage_Request, CChatRoom_SendChatMessage_Response,
    CChatRoom_UpdateMessageReaction_Request, CChatRoom_UpdateMessageReaction_Response,
    EChatRoomMessageReactionType, cchat_room_delete_chat_messages_request,
    cchat_room_get_message_history_response,
};
use steam_vent_proto::steammessages_friendmessages_steamclient::{
    CFriendMessages_IncomingMessage_Notification, CFriendMessages_SendMessage_Request,
    CFriendMessages_SendMessage_Response,
};
use steamid_ng::SteamID;
use thiserror::Error;
use tokio::time::sleep;
use tokio_stream::{Stream, StreamExt};
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

/// A chat group and all chat rooms returned by `GetMyChatRoomGroups`.
#[derive(Debug, Clone)]
pub struct ChatGroupInfo {
    /// The unique identifier for the chat group.
    pub chat_group_id: u64,
    /// The display name of the chat group.
    pub chat_group_name: String,
    /// Chat rooms within this group.
    pub chats: Vec<ChatRoomInfo>,
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

/// Supported reaction types for group chat message reactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionType {
    /// Steam emoticon reaction (e.g. `:steamhappy:`).
    Emoticon,
    /// Steam sticker reaction.
    Sticker,
}

impl ReactionType {
    fn into_proto(self) -> EChatRoomMessageReactionType {
        match self {
            Self::Emoticon => EChatRoomMessageReactionType::k_EChatRoomMessageReactionType_Emoticon,
            Self::Sticker => EChatRoomMessageReactionType::k_EChatRoomMessageReactionType_Sticker,
        }
    }

    fn from_proto(value: EChatRoomMessageReactionType) -> Option<Self> {
        match value {
            EChatRoomMessageReactionType::k_EChatRoomMessageReactionType_Emoticon => {
                Some(Self::Emoticon)
            }
            EChatRoomMessageReactionType::k_EChatRoomMessageReactionType_Sticker => {
                Some(Self::Sticker)
            }
            EChatRoomMessageReactionType::k_EChatRoomMessageReactionType_Invalid => None,
        }
    }
}

/// A single reaction summary attached to a message history entry.
#[derive(Debug, Clone)]
pub struct MessageReactionInfo {
    /// Reaction category (emoticon or sticker).
    pub reaction_type: ReactionType,
    /// Raw reaction identifier (for emoticons this is usually `:name:`).
    pub reaction: String,
    /// Number of users who reacted with this reaction.
    pub num_reactors: u32,
    /// Whether the current user has reacted with this reaction.
    pub has_user_reacted: bool,
}

/// A chat message returned by history APIs, including reaction data.
#[derive(Debug, Clone)]
pub struct ChatMessageHistoryEntry {
    /// Message sender Steam ID.
    pub sender: SteamID,
    /// Server timestamp of the message.
    pub server_timestamp: u32,
    /// Message ordinal in channel history.
    pub ordinal: u32,
    /// Message text body.
    pub message: String,
    /// Whether this message is deleted.
    pub deleted: bool,
    /// Reactions currently associated with this message.
    pub reactions: Vec<MessageReactionInfo>,
}

/// A real-time reaction notification event.
#[derive(Debug, Clone)]
pub struct ReactionEvent {
    /// Chat group identifier.
    pub chat_group_id: u64,
    /// Chat room identifier.
    pub chat_id: u64,
    /// Message server timestamp.
    pub server_timestamp: u32,
    /// Message ordinal.
    pub ordinal: u32,
    /// Steam ID of the user who reacted.
    pub reactor: SteamID,
    /// Type of reaction.
    pub reaction_type: ReactionType,
    /// Raw reaction identifier.
    pub reaction: String,
    /// `true` for add, `false` for remove.
    pub is_add: bool,
}

impl EnhancedGroupChatMessage {
    /// Create an enhanced message from a notification, preserving the whole notification object
    pub fn from_notification(notification: &CChatRoom_IncomingChatMessage_Notification) -> Self {
        let preprocessed = MessagePreprocessor::preprocess_message(notification.message());
        Self {
            chat_group_id: notification.chat_group_id(),
            chat_id: notification.chat_id(),
            sender_steam_id: SteamID::from(notification.steamid_sender()),
            message: notification.message().to_string(),
            timestamp: notification.timestamp(),
            chat_name: notification.chat_name().to_string(),
            ordinal: notification.ordinal(),
            preprocessed,
        }
    }
}

impl MessageReactionInfo {
    fn from_proto(
        reaction: &cchat_room_get_message_history_response::chat_message::MessageReaction,
    ) -> Option<Self> {
        let reaction_type = ReactionType::from_proto(reaction.reaction_type())?;
        Some(Self {
            reaction_type,
            reaction: reaction.reaction().to_string(),
            num_reactors: reaction.num_reactors(),
            has_user_reacted: reaction.has_user_reacted(),
        })
    }
}

impl ChatMessageHistoryEntry {
    fn from_proto(message: &cchat_room_get_message_history_response::ChatMessage) -> Self {
        let mut reactions = Vec::new();
        for reaction in &message.reactions {
            match MessageReactionInfo::from_proto(reaction) {
                Some(reaction_info) => reactions.push(reaction_info),
                None => {
                    tracing::warn!(
                        reaction_type = ?reaction.reaction_type(),
                        reaction = reaction.reaction(),
                        "Skipping unsupported reaction type from history entry"
                    );
                }
            }
        }

        Self {
            sender: SteamID::from(message.sender() as u64),
            server_timestamp: message.server_timestamp(),
            ordinal: message.ordinal(),
            message: message.message().to_string(),
            deleted: message.deleted(),
            reactions,
        }
    }
}

impl ReactionEvent {
    fn from_notification(notification: &CChatRoom_MessageReaction_Notification) -> Option<Self> {
        let reaction_type = ReactionType::from_proto(notification.reaction_type())?;
        Some(Self {
            chat_group_id: notification.chat_group_id(),
            chat_id: notification.chat_id(),
            server_timestamp: notification.server_timestamp(),
            ordinal: notification.ordinal(),
            reactor: SteamID::from(notification.reactor()),
            reaction_type,
            reaction: notification.reaction().to_string(),
            is_add: notification.is_add(),
        })
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

    /// Get all chat groups the user belongs to, including every chat room in each group.
    ///
    /// Uses the `GetMyChatRoomGroups` response directly, which already includes
    /// per-group chat room lists in `group_summary.chat_rooms`.
    pub async fn get_my_chat_groups(&self) -> Result<Vec<ChatGroupInfo>, Box<dyn Error>> {
        self.groups().get_my_chat_groups().await
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

    /// Add a reaction to a group chat message.
    ///
    /// # Arguments
    ///
    /// * `chat_group_id` - The unique identifier for the chat group
    /// * `chat_id` - The unique identifier for the specific chat room
    /// * `server_timestamp` - The message server timestamp
    /// * `ordinal` - The message ordinal (can be 0)
    /// * `reaction_type` - Reaction category (emoticon or sticker)
    /// * `reaction` - Reaction value (e.g. `:steamhappy:` for emoticons)
    ///
    /// # Returns
    ///
    /// The updated number of reactors for this reaction.
    #[instrument(
        name = "kether.chat.add_message_reaction",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type)
    )]
    pub async fn add_message_reaction(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<u32, Box<dyn Error>> {
        self.messaging()
            .add_message_reaction(
                chat_group_id,
                chat_id,
                server_timestamp,
                ordinal,
                reaction_type,
                reaction,
            )
            .await
    }

    /// Remove a reaction from a group chat message.
    ///
    /// # Returns
    ///
    /// The updated number of reactors for this reaction after removal.
    #[instrument(
        name = "kether.chat.remove_message_reaction",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type)
    )]
    pub async fn remove_message_reaction(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<u32, Box<dyn Error>> {
        self.messaging()
            .remove_message_reaction(
                chat_group_id,
                chat_id,
                server_timestamp,
                ordinal,
                reaction_type,
                reaction,
            )
            .await
    }

    /// List users who reacted with a specific reaction on a message.
    ///
    /// # Arguments
    ///
    /// * `limit` - Optional max number of reactors to return
    ///
    /// # Returns
    ///
    /// A list of reactor Steam IDs.
    #[instrument(
        name = "kether.chat.get_message_reaction_reactors",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type, limit = ?limit)
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn get_message_reaction_reactors(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
        limit: Option<u32>,
    ) -> Result<Vec<SteamID>, Box<dyn Error>> {
        self.messaging()
            .get_message_reaction_reactors(
                chat_group_id,
                chat_id,
                server_timestamp,
                ordinal,
                reaction_type,
                reaction,
                limit,
            )
            .await
    }

    /// Get message history for a chat room, including per-message reaction summaries.
    #[instrument(
        name = "kether.chat.get_message_history",
        skip(self),
        fields(chat_group_id, chat_id, max_count = ?max_count)
    )]
    pub async fn get_message_history(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        max_count: Option<u32>,
    ) -> Result<Vec<ChatMessageHistoryEntry>, Box<dyn Error>> {
        self.messaging()
            .get_message_history(chat_group_id, chat_id, max_count)
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
    pub async fn get_chat_room_state(
        &self,
        chat_group_id: u64,
    ) -> Result<CChatRoom_GetChatRoomGroupState_Response, Box<dyn Error>> {
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
        self.notifications()
            .listen_for_group_messages(callback)
            .await
    }

    /// Listen for incoming message reaction events.
    ///
    /// # Arguments
    ///
    /// * `callback` - A closure that will be called for each reaction event
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback panics.
    pub async fn listen_for_reactions<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(ReactionEvent) + Send + 'static,
    {
        self.notifications().listen_for_reactions(callback).await
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
        self.notifications()
            .listen_for_friend_messages(callback)
            .await
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
                Ok(item) => handler(item)
                    .map_err(|source| NotificationDispatchError::Callback { source })?,
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
    fn chats_from_group_summary(
        summary: &steam_vent_proto::steammessages_chat_steamclient::CChatRoom_GetChatRoomGroupSummary_Response,
    ) -> Vec<ChatRoomInfo> {
        let group_id = summary.chat_group_id();
        let group_name = summary.chat_group_name().to_string();
        let chats: Vec<ChatRoomInfo> = summary
            .chat_rooms
            .iter()
            .map(|room| ChatRoomInfo {
                chat_group_id: group_id,
                chat_id: room.chat_id(),
                chat_name: room.chat_name().to_string(),
                chat_group_name: group_name.clone(),
                is_joined: true,
            })
            .collect();

        if chats.is_empty() {
            vec![ChatRoomInfo {
                chat_group_id: group_id,
                chat_id: summary.default_chat_id(),
                chat_name: group_name.clone(),
                chat_group_name: group_name,
                is_joined: true,
            }]
        } else {
            chats
        }
    }

    /// Get all chat groups the user belongs to, including every chat room in each group.
    pub async fn get_my_chat_groups(&self) -> Result<Vec<ChatGroupInfo>, Box<dyn Error>> {
        let req = CChatRoom_GetMyChatRoomGroups_Request::new();
        let response: CChatRoom_GetMyChatRoomGroups_Response =
            self.connection.service_method(req).await?;

        let mut groups = Vec::new();
        for pair in &response.chat_room_groups {
            if let Some(summary) = pair.group_summary.as_ref() {
                groups.push(ChatGroupInfo {
                    chat_group_id: summary.chat_group_id(),
                    chat_group_name: summary.chat_group_name().to_string(),
                    chats: Self::chats_from_group_summary(summary),
                });
            }
        }

        Ok(groups)
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
        let final_preprocessed = Self::process_send_message_response(&params, &response);

        // According to DrMcKay's wiki, the response has both server_timestamp and ordinal.
        // Ordinal can be 0 (and can be omitted in deletion requests if 0).
        // The response values are sufficient for deletion, so we don't need to wait for the notification.
        // This avoids unnecessary delays.

        debug!(
            chat_group_id = params.chat_group_id,
            chat_id = params.chat_id,
            ordinal = final_preprocessed.ordinal.unwrap_or(0),
            "group message dispatched"
        );

        Ok(final_preprocessed)
    }

    fn build_send_message_request(
        params: &SendGroupMessageParams,
    ) -> CChatRoom_SendChatMessage_Request {
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

    fn ensure_valid_message_identifier(
        server_timestamp: u32,
        ordinal: u32,
    ) -> Result<(), Box<dyn Error>> {
        if server_timestamp == 0 {
            return Err(format!(
                "Invalid message identifier for reaction: server_timestamp={} (must be non-zero). Ordinal={} is allowed to be 0.",
                server_timestamp, ordinal
            ).into());
        }
        Ok(())
    }

    fn build_update_message_reaction_request(
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
        is_add: bool,
    ) -> Result<CChatRoom_UpdateMessageReaction_Request, Box<dyn Error>> {
        Self::ensure_valid_message_identifier(server_timestamp, ordinal)?;

        let mut req = CChatRoom_UpdateMessageReaction_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);
        req.set_server_timestamp(server_timestamp);
        req.set_ordinal(ordinal);
        req.set_reaction_type(reaction_type.into_proto());
        req.set_reaction(reaction.to_string());
        req.set_is_add(is_add);
        Ok(req)
    }

    /// Add a reaction to a group chat message.
    #[instrument(
        name = "kether.chat.add_message_reaction",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type)
    )]
    pub async fn add_message_reaction(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<u32, Box<dyn Error>> {
        let req = Self::build_update_message_reaction_request(
            chat_group_id,
            chat_id,
            server_timestamp,
            ordinal,
            reaction_type,
            reaction,
            true,
        )?;
        let response: CChatRoom_UpdateMessageReaction_Response =
            self.connection.service_method(req).await?;

        debug!(
            chat_group_id,
            chat_id,
            server_timestamp,
            ordinal,
            reaction_type = ?reaction_type,
            num_reactors = response.num_reactors(),
            "message reaction added"
        );

        Ok(response.num_reactors())
    }

    /// Remove a reaction from a group chat message.
    #[instrument(
        name = "kether.chat.remove_message_reaction",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type)
    )]
    pub async fn remove_message_reaction(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<u32, Box<dyn Error>> {
        let req = Self::build_update_message_reaction_request(
            chat_group_id,
            chat_id,
            server_timestamp,
            ordinal,
            reaction_type,
            reaction,
            false,
        )?;
        let response: CChatRoom_UpdateMessageReaction_Response =
            self.connection.service_method(req).await?;

        debug!(
            chat_group_id,
            chat_id,
            server_timestamp,
            ordinal,
            reaction_type = ?reaction_type,
            num_reactors = response.num_reactors(),
            "message reaction removed"
        );

        Ok(response.num_reactors())
    }

    /// List users who reacted with a specific reaction on a message.
    #[instrument(
        name = "kether.chat.get_message_reaction_reactors",
        skip(self, reaction),
        fields(chat_group_id, chat_id, server_timestamp, ordinal, reaction_type = ?reaction_type, limit = ?limit)
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn get_message_reaction_reactors(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
        limit: Option<u32>,
    ) -> Result<Vec<SteamID>, Box<dyn Error>> {
        Self::ensure_valid_message_identifier(server_timestamp, ordinal)?;

        let mut req = CChatRoom_GetMessageReactionReactors_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);
        req.set_server_timestamp(server_timestamp);
        req.set_ordinal(ordinal);
        req.set_reaction_type(reaction_type.into_proto());
        req.set_reaction(reaction.to_string());
        if let Some(limit) = limit {
            req.set_limit(limit);
        }

        let response: CChatRoom_GetMessageReactionReactors_Response =
            self.connection.service_method(req).await?;
        let reactors: Vec<SteamID> = response
            .reactors
            .iter()
            .map(|account_id| SteamID::from(*account_id as u64))
            .collect();

        debug!(
            chat_group_id,
            chat_id,
            server_timestamp,
            ordinal,
            reaction_type = ?reaction_type,
            reactor_count = reactors.len(),
            "message reaction reactors fetched"
        );

        Ok(reactors)
    }

    /// Fetch message history for a chat room, including aggregated reaction summaries.
    #[instrument(
        name = "kether.chat.get_message_history",
        skip(self),
        fields(chat_group_id, chat_id, max_count = ?max_count)
    )]
    pub async fn get_message_history(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        max_count: Option<u32>,
    ) -> Result<Vec<ChatMessageHistoryEntry>, Box<dyn Error>> {
        let mut req = CChatRoom_GetMessageHistory_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);
        if let Some(max_count) = max_count {
            req.set_max_count(max_count);
        }

        let response: CChatRoom_GetMessageHistory_Response =
            self.connection.service_method(req).await?;
        let history_entries: Vec<ChatMessageHistoryEntry> = response
            .messages
            .iter()
            .map(ChatMessageHistoryEntry::from_proto)
            .collect();

        debug!(
            chat_group_id,
            chat_id,
            message_count = history_entries.len(),
            more_available = response.more_available(),
            "chat message history fetched"
        );

        Ok(history_entries)
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

        // Log what we're trying to delete for debugging
        let timestamps: Vec<u32> = messages.iter().map(|(ts, _)| *ts).collect();
        let ordinals: Vec<u32> = messages.iter().map(|(_, ord)| *ord).collect();
        debug!(
            chat_group_id,
            chat_id,
            message_count,
            ?timestamps,
            ?ordinals,
            "Attempting to delete messages"
        );

        let mut req = CChatRoom_DeleteChatMessages_Request::new();
        req.set_chat_group_id(chat_group_id);
        req.set_chat_id(chat_id);

        // Convert (server_timestamp, ordinal) tuples to Message structs
        // According to DrMcKay's wiki, ordinal can be omitted if 0, but server_timestamp must be non-zero
        for (server_timestamp, ordinal) in messages {
            // Validate server_timestamp - it must be non-zero
            if server_timestamp == 0 {
                tracing::error!(
                    server_timestamp,
                    ordinal,
                    "Cannot delete message: server_timestamp is zero (invalid)"
                );
                return Err(format!(
                    "Invalid message identifier for deletion: server_timestamp={} (must be non-zero). Ordinal={} is allowed to be 0.",
                    server_timestamp, ordinal
                ).into());
            }
            // Ordinal can be 0 (it can be omitted in deletion requests per DrMcKay's wiki)

            let mut msg = cchat_room_delete_chat_messages_request::Message::new();
            msg.set_server_timestamp(server_timestamp);
            msg.set_ordinal(ordinal);

            // Verify both fields are actually set in the protobuf message
            if !msg.has_server_timestamp() || !msg.has_ordinal() {
                tracing::error!(
                    server_timestamp,
                    ordinal,
                    "Message fields not properly set in protobuf for deletion"
                );
                return Err(format!(
                    "Failed to set message fields in protobuf: server_timestamp={}, ordinal={}",
                    server_timestamp, ordinal
                )
                .into());
            }

            req.messages.push(msg);
        }

        let response: CChatRoom_DeleteChatMessages_Response =
            self.connection.service_method(req).await?;

        debug!(
            chat_group_id,
            chat_id, message_count, "group messages deleted"
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
                (Some(ts), Some(ord)) if ts > 0 => {
                    // server_timestamp must be non-zero, but ordinal can be 0 (per DrMcKay's wiki)
                    message_identifiers.push((ts, ord));
                }
                (Some(ts), Some(ord)) if ts == 0 => {
                    skipped_count += 1;
                    tracing::warn!(
                        server_timestamp = ts,
                        ordinal = ord,
                        "Skipping message deletion: server_timestamp is zero (invalid)"
                    );
                }
                (Some(ts), None) if ts > 0 => {
                    // server_timestamp is valid, ordinal is None - use 0 (ordinal can be omitted if 0)
                    message_identifiers.push((ts, 0));
                }
                _ => {
                    skipped_count += 1;
                    tracing::warn!("Skipping message deletion: missing server_timestamp");
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

    /// Listen for incoming reaction events with error handling.
    ///
    /// The callback can return an error to stop the listener, or `Ok(())` to continue.
    pub async fn listen_for_reactions_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(ReactionEvent) -> CallbackResult + Send + 'static,
    {
        let mut user_callback = callback;
        self.reaction_stream()
            .for_each(
                move |notification| match ReactionEvent::from_notification(&notification) {
                    Some(event) => user_callback(event),
                    None => {
                        tracing::warn!(
                            chat_group_id = notification.chat_group_id(),
                            chat_id = notification.chat_id(),
                            server_timestamp = notification.server_timestamp(),
                            ordinal = notification.ordinal(),
                            reaction_type = ?notification.reaction_type(),
                            "Skipping unsupported reaction notification type"
                        );
                        Ok(())
                    }
                },
            )
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(err) })
    }

    /// Listen for incoming reaction events.
    ///
    /// This is a convenience wrapper that ignores callback errors.
    pub async fn listen_for_reactions<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(ReactionEvent) + Send + 'static,
    {
        self.listen_for_reactions_with(move |event| {
            callback(event);
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
    pub async fn listen_for_friend_messages_with<F>(
        &self,
        callback: F,
    ) -> Result<(), Box<dyn Error>>
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

    fn group_stream(&self) -> NotificationStream<'_, CChatRoom_IncomingChatMessage_Notification> {
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

    fn reaction_stream(&self) -> NotificationStream<'_, CChatRoom_MessageReaction_Notification> {
        let stream = self
            .connection
            .on_notification::<CChatRoom_MessageReaction_Notification>()
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
        let logon = match (
            std::env::var("STEAM_ACCOUNT"),
            std::env::var("STEAM_PASSWORD"),
        ) {
            (Ok(account), Ok(password)) => {
                println!("Using credentials from environment variables");
                LogOn::new(&account, &password).await
            }
            _ => {
                println!("No credentials provided, using anonymous connection");
                LogOn::new_anonymous().await
            }
        }
        .unwrap();

        let chat_client = ChatRoomClient::new(logon.connection().clone());

        // Test getting chat rooms (should work even if empty)
        let chat_rooms = chat_client.get_my_chat_rooms().await;
        assert!(chat_rooms.is_ok(), "Should be able to get chat rooms");

        let rooms = chat_rooms.unwrap();
        println!("Found {} chat rooms:", rooms.len());

        for (i, room) in rooms.iter().enumerate() {
            println!(
                "  {}. {} (Group: {})",
                i + 1,
                room.chat_name,
                room.chat_group_name
            );
            println!(
                "     Group ID: {}, Chat ID: {}",
                room.chat_group_id, room.chat_id
            );
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
