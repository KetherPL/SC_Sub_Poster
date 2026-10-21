// SPDX-License-Identifier: LGPL-3.0-only

//! Private (friend-to-friend) messaging.
//!
//! Covers the whole `FriendMessages` service: sending messages and typing indicators,
//! marking conversations as read, message history, the list of active conversations,
//! reactions on private messages, and listeners for incoming messages, typing, conversation
//! changes and reactions.
//!
//! Formatting rules are the same as in group chats (see [`crate::preprocessing`]): use
//! `/me`, `/code`, ... slash commands, `:emoticon:` text and plain URLs; only mentions are
//! sent as BBCode.
//!
//! Not supported with `steam-vent` 0.5: read-receipt echoes from your other sessions
//! (`FriendMessagesClient.NotifyAckMessageEcho#1`), because that notification reuses the
//! `FriendMessages.AckMessage#1` message type and cannot be subscribed to separately.

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_friendmessages_steamclient::{
    CFriendMessages_AckMessage_Notification, CFriendMessages_GetRecentMessages_Request,
    CFriendMessages_IncomingMessage_Notification, CFriendMessages_IsInFriendsUIBeta_Request,
    CFriendMessages_MessageReaction_Notification, CFriendMessages_SendMessage_Request,
    CFriendMessages_SendMessage_Response, CFriendMessages_UpdateMessageReaction_Request,
    CFriendsMessages_GetActiveMessageSessions_Request, EMessageReactionType,
    cfriend_messages_get_recent_messages_response,
};
use steamid_ng::SteamID;
use tracing::{debug, instrument};

use crate::chatroom::{
    CallbackResult, ChatRoomClient, ReactionType, listen_notifications, steam_id_from_account_id,
};
use crate::rpc;

/// Steam's `EChatEntryType`: what a private-message entry represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatEntryType {
    /// A normal chat message.
    ChatMessage,
    /// The friend is typing.
    Typing,
    /// A game invite.
    InviteGame,
    /// An emote (legacy; `/me` is sent as a normal message nowadays).
    Emote,
    /// The friend closed the conversation window.
    LeftConversation,
    /// The friend entered the conversation.
    Entered,
    /// Kicked from the conversation.
    WasKicked,
    /// Banned from the conversation.
    WasBanned,
    /// The friend disconnected.
    Disconnected,
    /// A message replayed from history.
    HistoricalChat,
    /// A link was blocked by Steam.
    LinkBlocked,
    /// A value not known to this crate.
    Other(i32),
}

impl ChatEntryType {
    /// The raw `EChatEntryType` value.
    pub fn as_i32(self) -> i32 {
        match self {
            Self::ChatMessage => 1,
            Self::Typing => 2,
            Self::InviteGame => 3,
            Self::Emote => 4,
            Self::LeftConversation => 6,
            Self::Entered => 7,
            Self::WasKicked => 8,
            Self::WasBanned => 9,
            Self::Disconnected => 10,
            Self::HistoricalChat => 11,
            Self::LinkBlocked => 14,
            Self::Other(value) => value,
        }
    }

    /// Maps a raw `EChatEntryType` value.
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => Self::ChatMessage,
            2 => Self::Typing,
            3 => Self::InviteGame,
            4 => Self::Emote,
            6 => Self::LeftConversation,
            7 => Self::Entered,
            8 => Self::WasKicked,
            9 => Self::WasBanned,
            10 => Self::Disconnected,
            11 => Self::HistoricalChat,
            14 => Self::LinkBlocked,
            other => Self::Other(other),
        }
    }
}

fn reaction_type_to_proto(reaction_type: ReactionType) -> EMessageReactionType {
    match reaction_type {
        ReactionType::Emoticon => EMessageReactionType::k_EMessageReactionType_Emoticon,
        ReactionType::Sticker => EMessageReactionType::k_EMessageReactionType_Sticker,
    }
}

fn reaction_type_from_proto(reaction_type: EMessageReactionType) -> Option<ReactionType> {
    match reaction_type {
        EMessageReactionType::k_EMessageReactionType_Emoticon => Some(ReactionType::Emoticon),
        EMessageReactionType::k_EMessageReactionType_Sticker => Some(ReactionType::Sticker),
        EMessageReactionType::k_EMessageReactionType_Invalid => None,
    }
}

/// Options for sending a private message. Defaults match the official client.
#[derive(Debug, Clone)]
pub struct SendFriendMessageOptions {
    /// What the entry represents (default: [`ChatEntryType::ChatMessage`]).
    pub entry_type: ChatEntryType,
    /// Whether the text may contain BBCode such as mentions (default: `true`).
    pub contains_bbcode: bool,
    /// Echo the message to your other logged-in sessions (default: `true`).
    pub echo_to_sender: bool,
    /// Low-priority delivery (default: `false`).
    pub low_priority: bool,
    /// Optional client-side message id for de-duplication.
    pub client_message_id: Option<String>,
}

impl Default for SendFriendMessageOptions {
    fn default() -> Self {
        Self {
            entry_type: ChatEntryType::ChatMessage,
            contains_bbcode: true,
            echo_to_sender: true,
            low_priority: false,
            client_message_id: None,
        }
    }
}

/// Result of sending a private message.
#[derive(Debug, Clone)]
pub struct SentFriendMessage {
    /// The message as stored by Steam (e.g. a `/pre` command expanded to `[pre]...[/pre]`).
    pub modified_message: String,
    /// Plain-text version of the message without BBCode.
    pub message_without_bbcode: String,
    /// Server timestamp of the message (identifies it together with `ordinal`).
    pub server_timestamp: u32,
    /// Ordinal of the message within `server_timestamp`.
    pub ordinal: u32,
}

impl From<CFriendMessages_SendMessage_Response> for SentFriendMessage {
    fn from(response: CFriendMessages_SendMessage_Response) -> Self {
        Self {
            modified_message: response.modified_message().to_string(),
            message_without_bbcode: response.message_without_bb_code().to_string(),
            server_timestamp: response.server_timestamp(),
            ordinal: response.ordinal(),
        }
    }
}

/// An incoming private-message event: a message, a typing indicator, the friend leaving the
/// conversation, etc. (see [`FriendMessageEvent::entry_type`]).
#[derive(Debug, Clone)]
pub struct FriendMessageEvent {
    /// The friend on the other side of the conversation.
    pub friend: SteamID,
    /// What this entry represents.
    pub entry_type: ChatEntryType,
    /// Message text with BBCode (empty for typing and similar events).
    pub message: String,
    /// Message text without BBCode.
    pub message_no_bbcode: String,
    /// Server timestamp.
    pub server_timestamp: u32,
    /// Ordinal within `server_timestamp`.
    pub ordinal: u32,
    /// `true` when this is an echo of a message *you* sent from another session; `friend`
    /// is then the recipient.
    pub local_echo: bool,
    /// Whether the sender is a limited Steam account.
    pub from_limited_account: bool,
    /// Whether the message was sent with low priority.
    pub low_priority: bool,
}

impl FriendMessageEvent {
    fn from_notification(notification: &CFriendMessages_IncomingMessage_Notification) -> Self {
        Self {
            friend: SteamID::from(notification.steamid_friend()),
            entry_type: ChatEntryType::from_i32(notification.chat_entry_type()),
            message: notification.message().to_string(),
            message_no_bbcode: notification.message_no_bbcode().to_string(),
            server_timestamp: notification.rtime32_server_timestamp(),
            ordinal: notification.ordinal(),
            local_echo: notification.local_echo(),
            from_limited_account: notification.from_limited_account(),
            low_priority: notification.low_priority(),
        }
    }
}

/// A reaction summary on a private message from history.
#[derive(Debug, Clone)]
pub struct FriendMessageReaction {
    /// Reaction category (emoticon or sticker).
    pub reaction_type: ReactionType,
    /// Raw reaction identifier (for emoticons usually `:name:`).
    pub reaction: String,
    /// Users who reacted with it.
    pub reactors: Vec<SteamID>,
}

/// A private message from history.
#[derive(Debug, Clone)]
pub struct FriendHistoryMessage {
    /// Who sent the message (you or the friend).
    pub sender: SteamID,
    /// Server timestamp.
    pub server_timestamp: u32,
    /// Ordinal within `server_timestamp`.
    pub ordinal: u32,
    /// Message text.
    pub message: String,
    /// Reactions on the message.
    pub reactions: Vec<FriendMessageReaction>,
}

impl FriendHistoryMessage {
    fn from_proto(message: &cfriend_messages_get_recent_messages_response::FriendMessage) -> Self {
        let reactions = message
            .reactions
            .iter()
            .filter_map(|reaction| {
                Some(FriendMessageReaction {
                    reaction_type: reaction_type_from_proto(reaction.reaction_type())?,
                    reaction: reaction.reaction().to_string(),
                    reactors: reaction
                        .reactors
                        .iter()
                        .map(|&account_id| steam_id_from_account_id(account_id))
                        .collect(),
                })
            })
            .collect();
        Self {
            sender: steam_id_from_account_id(message.accountid()),
            server_timestamp: message.timestamp(),
            ordinal: message.ordinal(),
            message: message.message().to_string(),
            reactions,
        }
    }
}

/// A page of private-message history (newest first).
#[derive(Debug, Clone)]
pub struct FriendHistoryPage {
    /// The messages in this page.
    pub messages: Vec<FriendHistoryMessage>,
    /// Whether older messages are available (request them with
    /// [`FriendHistoryQuery::before`]).
    pub more_available: bool,
}

/// Which part of a private conversation's history to fetch.
#[derive(Debug, Clone)]
pub struct FriendHistoryQuery {
    /// Maximum number of messages (default: 50).
    pub count: u32,
    /// Only the most recent conversation session (default: `false`).
    pub most_recent_conversation: bool,
    /// Return messages with BBCode (default: `true`).
    pub bbcode_format: bool,
    /// Only messages at or after this Unix time.
    pub start_time: Option<u32>,
    /// Start ordinal paired with `start_time`.
    pub start_ordinal: Option<u32>,
    /// Paging: return messages older than this `(server_timestamp, ordinal)`.
    pub before: Option<(u32, u32)>,
}

impl Default for FriendHistoryQuery {
    fn default() -> Self {
        Self {
            count: 50,
            most_recent_conversation: false,
            bbcode_format: true,
            start_time: None,
            start_ordinal: None,
            before: None,
        }
    }
}

/// A private conversation with recent activity.
#[derive(Debug, Clone)]
pub struct FriendMessageSession {
    /// The friend.
    pub friend: SteamID,
    /// Time of the last message.
    pub last_message: u32,
    /// Time you last viewed the conversation.
    pub last_view: u32,
    /// Unread messages in the conversation.
    pub unread_message_count: u32,
}

/// Active private conversations.
#[derive(Debug, Clone)]
pub struct FriendMessageSessions {
    /// The conversations.
    pub sessions: Vec<FriendMessageSession>,
    /// Server time of the response; pass it as `since` next time to get only changes.
    pub timestamp: u32,
}

/// A real-time reaction on a private message.
#[derive(Debug, Clone)]
pub struct FriendReactionEvent {
    /// The friend on the other side of the conversation.
    pub friend: SteamID,
    /// Server timestamp of the message reacted to.
    pub server_timestamp: u32,
    /// Ordinal of the message reacted to.
    pub ordinal: u32,
    /// Who reacted.
    pub reactor: SteamID,
    /// Reaction category.
    pub reaction_type: ReactionType,
    /// Raw reaction identifier.
    pub reaction: String,
    /// `true` for add, `false` for remove.
    pub is_add: bool,
}

/// Whether a user uses the new Steam chat ("friends UI").
#[derive(Debug, Clone, Copy)]
pub struct FriendsUiStatus {
    /// Currently online in the new friends UI.
    pub online_in_friends_ui: bool,
    /// Has ever used the new friends UI.
    pub has_used_friends_ui: bool,
}

/// Private messaging operations. Get one with [`ChatRoomClient::friends`].
pub struct FriendMessaging<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access private (friend-to-friend) messaging.
    pub fn friends(&self) -> FriendMessaging<'_> {
        FriendMessaging {
            connection: self.connection(),
        }
    }
}

impl<'a> FriendMessaging<'a> {
    /// Send a private message with the official client's defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if Steam rejects the message or the request fails.
    pub async fn send_message(
        &self,
        friend: SteamID,
        message: &str,
    ) -> Result<SentFriendMessage, Box<dyn Error>> {
        self.send_message_with(friend, message, SendFriendMessageOptions::default())
            .await
    }

    /// Send a private message with explicit options.
    ///
    /// # Errors
    ///
    /// Returns an error if Steam rejects the message or the request fails.
    #[instrument(
        name = "kether.friends.send_message",
        skip(self, message, options),
        fields(friend = %friend.steam3(), entry_type = ?options.entry_type)
    )]
    pub async fn send_message_with(
        &self,
        friend: SteamID,
        message: &str,
        options: SendFriendMessageOptions,
    ) -> Result<SentFriendMessage, Box<dyn Error>> {
        let mut request = CFriendMessages_SendMessage_Request::new();
        request.set_steamid(friend.into());
        request.set_chat_entry_type(options.entry_type.as_i32());
        request.set_message(message.to_string());
        request.set_contains_bbcode(options.contains_bbcode);
        request.set_echo_to_sender(options.echo_to_sender);
        request.set_low_priority(options.low_priority);
        if let Some(id) = options.client_message_id {
            request.set_client_message_id(id);
        }
        let response = self.connection.service_method(request).await?;
        debug!("friend message dispatched");
        Ok(response.into())
    }

    /// Show the "is typing" indicator to a friend (it disappears after a few seconds unless
    /// sent again).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn send_typing(&self, friend: SteamID) -> Result<(), Box<dyn Error>> {
        let options = SendFriendMessageOptions {
            entry_type: ChatEntryType::Typing,
            contains_bbcode: false,
            ..SendFriendMessageOptions::default()
        };
        self.send_message_with(friend, "", options).await?;
        Ok(())
    }

    /// Mark a conversation as read up to `timestamp` (use the latest message's server
    /// timestamp). Steam sends no response to this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the message could not be sent.
    pub async fn ack_message(&self, friend: SteamID, timestamp: u32) -> Result<(), Box<dyn Error>> {
        let mut request = CFriendMessages_AckMessage_Notification::new();
        request.set_steamid_partner(friend.into());
        request.set_timestamp(timestamp);
        rpc::notify(self.connection, request).await?;
        Ok(())
    }

    /// Fetch message history of the conversation with `friend` (newest first).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_message_history(
        &self,
        friend: SteamID,
        query: FriendHistoryQuery,
    ) -> Result<FriendHistoryPage, Box<dyn Error>> {
        let mut request = CFriendMessages_GetRecentMessages_Request::new();
        request.set_steamid1(u64::from(self.connection.steam_id()));
        request.set_steamid2(friend.into());
        request.set_count(query.count);
        request.set_most_recent_conversation(query.most_recent_conversation);
        request.set_bbcode_format(query.bbcode_format);
        if let Some(start_time) = query.start_time {
            request.set_rtime32_start_time(start_time);
        }
        if let Some(start_ordinal) = query.start_ordinal {
            request.set_start_ordinal(start_ordinal);
        }
        if let Some((time_last, ordinal_last)) = query.before {
            request.set_time_last(time_last);
            request.set_ordinal_last(ordinal_last);
        }
        let response = self.connection.service_method(request).await?;
        Ok(FriendHistoryPage {
            messages: response
                .messages
                .iter()
                .map(FriendHistoryMessage::from_proto)
                .collect(),
            more_available: response.more_available(),
        })
    }

    /// List private conversations with recent activity.
    ///
    /// * `since` - only conversations with messages after this time (e.g. the `timestamp` of
    ///   a previous call); `None` for all.
    /// * `only_with_messages` - skip conversations without messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_active_sessions(
        &self,
        since: Option<u32>,
        only_with_messages: bool,
    ) -> Result<FriendMessageSessions, Box<dyn Error>> {
        let mut request = CFriendsMessages_GetActiveMessageSessions_Request::new();
        if let Some(since) = since {
            request.set_lastmessage_since(since);
        }
        request.set_only_sessions_with_messages(only_with_messages);
        let response = self.connection.service_method(request).await?;
        Ok(FriendMessageSessions {
            sessions: response
                .message_sessions
                .iter()
                .map(|session| FriendMessageSession {
                    friend: steam_id_from_account_id(session.accountid_friend()),
                    last_message: session.last_message(),
                    last_view: session.last_view(),
                    unread_message_count: session.unread_message_count(),
                })
                .collect(),
            timestamp: response.timestamp(),
        })
    }

    /// Add a reaction to a private message. Returns everyone who reacted with it.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn add_reaction(
        &self,
        friend: SteamID,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<Vec<SteamID>, Box<dyn Error>> {
        self.update_reaction(friend, server_timestamp, ordinal, reaction_type, reaction, true)
            .await
    }

    /// Remove your reaction from a private message. Returns everyone still reacting with it.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn remove_reaction(
        &self,
        friend: SteamID,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
    ) -> Result<Vec<SteamID>, Box<dyn Error>> {
        self.update_reaction(friend, server_timestamp, ordinal, reaction_type, reaction, false)
            .await
    }

    async fn update_reaction(
        &self,
        friend: SteamID,
        server_timestamp: u32,
        ordinal: u32,
        reaction_type: ReactionType,
        reaction: &str,
        is_add: bool,
    ) -> Result<Vec<SteamID>, Box<dyn Error>> {
        let mut request = CFriendMessages_UpdateMessageReaction_Request::new();
        request.set_steamid(friend.into());
        request.set_server_timestamp(server_timestamp);
        request.set_ordinal(ordinal);
        request.set_reaction_type(reaction_type_to_proto(reaction_type));
        request.set_reaction(reaction.to_string());
        request.set_is_add(is_add);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .reactors
            .iter()
            .map(|&account_id| steam_id_from_account_id(account_id))
            .collect())
    }

    /// Check whether a user is using the new Steam chat ("friends UI").
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_friends_ui_status(
        &self,
        user: SteamID,
    ) -> Result<FriendsUiStatus, Box<dyn Error>> {
        let mut request = CFriendMessages_IsInFriendsUIBeta_Request::new();
        request.set_steamid(user.into());
        let response = self.connection.service_method(request).await?;
        Ok(FriendsUiStatus {
            online_in_friends_ui: response.online_in_friendsui(),
            has_used_friends_ui: response.has_used_friendsui(),
        })
    }

    /// Listen for every incoming private-message event: messages, typing indicators,
    /// friends leaving the conversation, and echoes of your own messages from other
    /// sessions (`local_echo`). Return `Err` from the callback to stop listening.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_events_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessageEvent) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: CFriendMessages_IncomingMessage_Notification| {
                callback(FriendMessageEvent::from_notification(&notification))
            },
        )
        .await
    }

    /// Listen only for actual messages from friends (no typing indicators, no echoes of your
    /// own messages). Return `Err` from the callback to stop listening.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_messages_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendMessageEvent) -> CallbackResult + Send + 'static,
    {
        self.listen_for_events_with(move |event| {
            if event.entry_type == ChatEntryType::ChatMessage && !event.local_echo {
                callback(event)
            } else {
                Ok(())
            }
        })
        .await
    }

    /// Listen for reactions added to or removed from private messages. Return `Err` from the
    /// callback to stop listening.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_reactions_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(FriendReactionEvent) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: CFriendMessages_MessageReaction_Notification| {
                match reaction_type_from_proto(notification.reaction_type()) {
                    Some(reaction_type) => callback(FriendReactionEvent {
                        friend: SteamID::from(notification.steamid_friend()),
                        server_timestamp: notification.server_timestamp(),
                        ordinal: notification.ordinal(),
                        reactor: SteamID::from(notification.reactor()),
                        reaction_type,
                        reaction: notification.reaction().to_string(),
                        is_add: notification.is_add(),
                    }),
                    None => Ok(()),
                }
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_entry_type_round_trips() {
        for value in [1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 14, 42] {
            assert_eq!(ChatEntryType::from_i32(value).as_i32(), value);
        }
        assert_eq!(ChatEntryType::from_i32(2), ChatEntryType::Typing);
        assert_eq!(ChatEntryType::from_i32(42), ChatEntryType::Other(42));
    }

    #[test]
    fn incoming_notification_maps_to_event() {
        let mut notification = CFriendMessages_IncomingMessage_Notification::new();
        notification.set_steamid_friend(76_561_198_000_000_001);
        notification.set_chat_entry_type(1);
        notification.set_message("[mention=1]@x[/mention] hi".to_string());
        notification.set_message_no_bbcode("@x hi".to_string());
        notification.set_rtime32_server_timestamp(1_700_000_000);
        notification.set_ordinal(2);
        notification.set_local_echo(true);
        let event = FriendMessageEvent::from_notification(&notification);
        assert_eq!(u64::from(event.friend), 76_561_198_000_000_001);
        assert_eq!(event.entry_type, ChatEntryType::ChatMessage);
        assert_eq!(event.message_no_bbcode, "@x hi");
        assert_eq!((event.server_timestamp, event.ordinal), (1_700_000_000, 2));
        assert!(event.local_echo);
    }

    #[test]
    fn history_message_uses_account_ids() {
        let mut message = cfriend_messages_get_recent_messages_response::FriendMessage::new();
        message.set_accountid(1_531_059_355);
        message.set_timestamp(10);
        message.set_ordinal(1);
        message.set_message("hey".to_string());
        let mut reaction =
            cfriend_messages_get_recent_messages_response::friend_message::MessageReaction::new();
        reaction.set_reaction_type(EMessageReactionType::k_EMessageReactionType_Emoticon);
        reaction.set_reaction(":steamhappy:".to_string());
        reaction.reactors.push(1_531_059_355);
        message.reactions.push(reaction);

        let parsed = FriendHistoryMessage::from_proto(&message);
        assert_eq!(parsed.sender.steam3(), "[U:1:1531059355]");
        assert_eq!(parsed.reactions.len(), 1);
        assert_eq!(parsed.reactions[0].reactors[0].steam3(), "[U:1:1531059355]");
    }
}
