// SPDX-License-Identifier: LGPL-3.0-only

//! Extra group chat notifications beyond incoming messages and reactions (which live in
//! [`crate::chatroom::ChatRoomNotifications`]).
//!
//! Get a handle with [`ChatRoomClient::events`]. Each listener runs until the stream ends,
//! the stream errors, or the callback returns `Err`. Several listeners can run at once.
//!
//! Not supported with `steam-vent` 0.5: read-marker echoes from your other sessions
//! (`ChatRoomClient.NotifyAckChatMessageEcho#1`), because that notification reuses the
//! `ChatRoom.AckChatMessage#1` message type and cannot be subscribed to separately.

use std::error::Error;

use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_ChatMessageModified_Notification, CChatRoom_ChatRoomGroupRoomsChange_Notification,
    CChatRoom_ChatRoomHeaderState_Notification, CChatRoom_MemberStateChange_Notification,
    ChatRoomClient_NotifyChatGroupUserStateChanged_Notification,
    ChatRoomClient_NotifyChatRoomDisconnect_Notification, EChatRoomJoinState,
};
use steamid_ng::SteamID;

use crate::chatroom::{CallbackResult, ChatRoomClient, listen_notifications, steam_id_from_account_id};

/// What happened to a group member (Steam's `EChatRoomMemberStateChange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChange {
    /// Joined the group.
    Joined,
    /// Left the group.
    Parted,
    /// Was kicked.
    Kicked,
    /// Was invited.
    Invited,
    /// Rank changed.
    RankChanged,
    /// Dismissed an invite.
    InviteDismissed,
    /// Was muted.
    Muted,
    /// Was banned.
    Banned,
    /// Roles changed.
    RolesChanged,
    /// A value not known to this crate.
    Other(i32),
}

impl MemberChange {
    /// Maps a raw `EChatRoomMemberStateChange` value.
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => Self::Joined,
            2 => Self::Parted,
            3 => Self::Kicked,
            4 => Self::Invited,
            7 => Self::RankChanged,
            8 => Self::InviteDismissed,
            9 => Self::Muted,
            10 => Self::Banned,
            12 => Self::RolesChanged,
            other => Self::Other(other),
        }
    }
}

/// A member's rank in a group (Steam's `EChatRoomGroupRank`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChatRank {
    /// Default / unknown.
    Default,
    /// Viewer.
    Viewer,
    /// Guest.
    Guest,
    /// Member.
    Member,
    /// Moderator.
    Moderator,
    /// Officer.
    Officer,
    /// Owner.
    Owner,
    /// A value not known to this crate.
    Other(i32),
}

impl ChatRank {
    /// Maps a raw `EChatRoomGroupRank` value.
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Default,
            10 => Self::Viewer,
            15 => Self::Guest,
            20 => Self::Member,
            30 => Self::Moderator,
            40 => Self::Officer,
            50 => Self::Owner,
            other => Self::Other(other),
        }
    }
}

/// A change to one group member.
#[derive(Debug, Clone)]
pub struct MemberStateChange {
    /// The group.
    pub chat_group_id: u64,
    /// The member.
    pub member: SteamID,
    /// What happened.
    pub change: MemberChange,
    /// The member's rank after the change.
    pub rank: ChatRank,
    /// The member's role ids after the change.
    pub role_ids: Vec<u64>,
    /// Whether the member is currently joined.
    pub is_joined: bool,
    /// When a kick expires (Unix time; `0` if not kicked).
    pub time_kick_expire: u32,
}

/// One message changed in a [`MessagesModified`] event.
#[derive(Debug, Clone, Copy)]
pub struct ModifiedMessage {
    /// Server timestamp of the message.
    pub server_timestamp: u32,
    /// Ordinal of the message.
    pub ordinal: u32,
    /// Whether the message was deleted.
    pub deleted: bool,
}

/// Messages in a room were changed (in practice: deleted).
#[derive(Debug, Clone)]
pub struct MessagesModified {
    /// The group.
    pub chat_group_id: u64,
    /// The room.
    pub chat_id: u64,
    /// The changed messages.
    pub messages: Vec<ModifiedMessage>,
}

/// Group chat event listeners. Get one with [`ChatRoomClient::events`].
pub struct ChatEvents<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access extra group chat notifications (member changes, deletions, rooms, header).
    pub fn events(&self) -> ChatEvents<'_> {
        ChatEvents {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatEvents<'a> {
    /// Listen for members joining, leaving, being kicked/banned/muted/invited, and rank or
    /// role changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_member_changes_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(MemberStateChange) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: CChatRoom_MemberStateChange_Notification| {
                let member = &notification.member;
                callback(MemberStateChange {
                    chat_group_id: notification.chat_group_id(),
                    member: steam_id_from_account_id(member.accountid()),
                    change: MemberChange::from_i32(notification.change() as i32),
                    rank: ChatRank::from_i32(member.rank() as i32),
                    role_ids: member.role_ids.clone(),
                    is_joined: member.state() == EChatRoomJoinState::k_EChatRoomJoinState_Joined,
                    time_kick_expire: member.time_kick_expire(),
                })
            },
        )
        .await
    }

    /// Listen for messages being modified (deleted) in rooms you are in.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_message_modifications_with<F>(
        &self,
        mut callback: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(MessagesModified) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: CChatRoom_ChatMessageModified_Notification| {
                callback(MessagesModified {
                    chat_group_id: notification.chat_group_id(),
                    chat_id: notification.chat_id(),
                    messages: notification
                        .messages
                        .iter()
                        .map(|message| ModifiedMessage {
                            server_timestamp: message.server_timestamp(),
                            ordinal: message.ordinal(),
                            deleted: message.deleted(),
                        })
                        .collect(),
                })
            },
        )
        .await
    }

    /// Listen for group header changes (name, tagline, avatar, owner, roles, broadcast),
    /// as the raw protobuf notification.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_header_changes_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(CChatRoom_ChatRoomHeaderState_Notification) -> CallbackResult + Send + 'static,
    {
        listen_notifications(self.connection, callback).await
    }

    /// Listen for a group's room list changing (rooms created, renamed, reordered, deleted),
    /// as the raw protobuf notification.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_rooms_changes_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(CChatRoom_ChatRoomGroupRoomsChange_Notification) -> CallbackResult + Send + 'static,
    {
        listen_notifications(self.connection, callback).await
    }

    /// Listen for changes to *your own* membership state in groups (joined, left, kicked,
    /// invited, notification settings), as the raw protobuf notification.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_own_state_changes_with<F>(&self, callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(ChatRoomClient_NotifyChatGroupUserStateChanged_Notification) -> CallbackResult
            + Send
            + 'static,
    {
        listen_notifications(self.connection, callback).await
    }

    /// Listen for Steam disconnecting you from groups' chat servers. The callback receives
    /// the affected group ids; rejoin or re-activate them (see
    /// `ChatRoomGroups::set_session_active_groups`) to keep receiving messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_disconnects_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(Vec<u64>) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: ChatRoomClient_NotifyChatRoomDisconnect_Notification| {
                callback(notification.chat_group_ids)
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_map_known_and_unknown_values() {
        assert_eq!(MemberChange::from_i32(1), MemberChange::Joined);
        assert_eq!(MemberChange::from_i32(10), MemberChange::Banned);
        assert_eq!(MemberChange::from_i32(99), MemberChange::Other(99));
        assert_eq!(ChatRank::from_i32(50), ChatRank::Owner);
        assert_eq!(ChatRank::from_i32(5), ChatRank::Other(5));
        assert!(ChatRank::Moderator < ChatRank::Owner);
    }
}
