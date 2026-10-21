// SPDX-License-Identifier: LGPL-3.0-only

//! Group chat moderation: kicking, banning, muting and managing pending invites.
//!
//! All calls need the matching permission in the group (see role actions in
//! [`crate::chat_roles`] when that feature is enabled).

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_GetBanList_Request, CChatRoom_GetInviteList_Request, CChatRoom_KickUser_Request,
    CChatRoom_MuteUser_Request, CChatRoom_RevokeInvite_Request, CChatRoom_SetUserBanState_Request,
};
use steamid_ng::SteamID;

use crate::chatroom::{ChatRoomClient, steam_id_from_account_id};

/// A ban in a chat group.
#[derive(Debug, Clone)]
pub struct ChatBan {
    /// The banned user.
    pub user: SteamID,
    /// Who issued the ban.
    pub banned_by: SteamID,
    /// When the ban was issued (Unix time).
    pub time_banned: u32,
    /// The ban reason, if any.
    pub reason: String,
}

/// A pending invite to a chat group.
#[derive(Debug, Clone)]
pub struct ChatGroupInvite {
    /// The invited user.
    pub invitee: SteamID,
    /// Who sent the invite.
    pub invited_by: SteamID,
    /// When the invite was sent (Unix time).
    pub time_invited: u32,
}

/// Moderation operations. Get one with [`ChatRoomClient::moderation`].
pub struct ChatModeration<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access group chat moderation (kick, ban, mute, invites).
    pub fn moderation(&self) -> ChatModeration<'_> {
        ChatModeration {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatModeration<'a> {
    /// Kick a user from a group. They can rejoin after `rejoin_after` (Unix time); `0` lets
    /// them rejoin immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn kick_user(
        &self,
        chat_group_id: u64,
        user: SteamID,
        rejoin_after: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_KickUser_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(user.into());
        request.set_expiration(rejoin_after as i32);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Ban a user from a group.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn ban_user(&self, chat_group_id: u64, user: SteamID) -> Result<(), Box<dyn Error>> {
        self.set_ban_state(chat_group_id, user, true).await
    }

    /// Lift a user's ban.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn unban_user(&self, chat_group_id: u64, user: SteamID) -> Result<(), Box<dyn Error>> {
        self.set_ban_state(chat_group_id, user, false).await
    }

    async fn set_ban_state(
        &self,
        chat_group_id: u64,
        user: SteamID,
        banned: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_SetUserBanState_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(user.into());
        request.set_ban_state(banned);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// List the group's bans.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn get_ban_list(&self, chat_group_id: u64) -> Result<Vec<ChatBan>, Box<dyn Error>> {
        let mut request = CChatRoom_GetBanList_Request::new();
        request.set_chat_group_id(chat_group_id);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .bans
            .iter()
            .map(|ban| ChatBan {
                user: steam_id_from_account_id(ban.accountid()),
                banned_by: steam_id_from_account_id(ban.accountid_actor()),
                time_banned: ban.time_banned(),
                reason: ban.ban_reason().to_string(),
            })
            .collect())
    }

    /// Mute a user in a group until `muted_until` (Unix time).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn mute_user(
        &self,
        chat_group_id: u64,
        user: SteamID,
        muted_until: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_MuteUser_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(user.into());
        request.set_expiration(muted_until as i32);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// List pending invites to the group.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_invite_list(
        &self,
        chat_group_id: u64,
    ) -> Result<Vec<ChatGroupInvite>, Box<dyn Error>> {
        let mut request = CChatRoom_GetInviteList_Request::new();
        request.set_chat_group_id(chat_group_id);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .invites
            .iter()
            .map(|invite| ChatGroupInvite {
                invitee: steam_id_from_account_id(invite.accountid()),
                invited_by: steam_id_from_account_id(invite.accountid_actor()),
                time_invited: invite.time_invited(),
            })
            .collect())
    }

    /// Revoke a pending invite.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn revoke_invite(&self, chat_group_id: u64, user: SteamID) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_RevokeInvite_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(user.into());
        self.connection.service_method(request).await?;
        Ok(())
    }
}
