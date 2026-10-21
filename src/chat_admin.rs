// SPDX-License-Identifier: LGPL-3.0-only

//! Group chat administration: creating and renaming groups, tagline, avatar, broadcast,
//! managing rooms (channels), and Steam-group (clan) chat privacy.

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_CreateChatRoomGroup_Request, CChatRoom_CreateChatRoomGroup_Response,
    CChatRoom_CreateChatRoom_Request, CChatRoom_DeleteChatRoom_Request,
    CChatRoom_RenameChatRoomGroup_Request, CChatRoom_RenameChatRoom_Request,
    CChatRoom_ReorderChatRoom_Request, CChatRoom_SaveChatRoomGroup_Request,
    CChatRoom_SetChatRoomGroupAvatar_Request, CChatRoom_SetChatRoomGroupTagline_Request,
    CChatRoom_SetChatRoomGroupWatchingBroadcast_Request, CChatRoomState,
    CClanChatRooms_SetClanChatRoomPrivate_Request,
};
use steamid_ng::SteamID;

use crate::chatroom::ChatRoomClient;

/// Group administration operations. Get one with [`ChatRoomClient::admin`].
pub struct ChatAdmin<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access group chat administration (groups, rooms, tagline, avatar).
    pub fn admin(&self) -> ChatAdmin<'_> {
        ChatAdmin {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatAdmin<'a> {
    /// Create a new chat group and invite `invitees`. An empty `name` creates an unnamed
    /// ("ad-hoc") group, which can be made permanent with [`ChatAdmin::save_group`].
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn create_group(
        &self,
        name: &str,
        invitees: &[SteamID],
    ) -> Result<CChatRoom_CreateChatRoomGroup_Response, Box<dyn Error>> {
        let mut request = CChatRoom_CreateChatRoomGroup_Request::new();
        if !name.is_empty() {
            request.set_name(name.to_string());
        }
        request.steamid_invitees = invitees.iter().map(|&id| u64::from(id)).collect();
        Ok(self.connection.service_method(request).await?)
    }

    /// Turn an unnamed group into a permanent, named group.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn save_group(&self, chat_group_id: u64, name: &str) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_SaveChatRoomGroup_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_name(name.to_string());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Rename a group. Returns the name as stored by Steam.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn rename_group(
        &self,
        chat_group_id: u64,
        name: &str,
    ) -> Result<String, Box<dyn Error>> {
        let mut request = CChatRoom_RenameChatRoomGroup_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_name(name.to_string());
        let response = self.connection.service_method(request).await?;
        Ok(response.name().to_string())
    }

    /// Set the group's tagline.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn set_tagline(&self, chat_group_id: u64, tagline: &str) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_SetChatRoomGroupTagline_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_tagline(tagline.to_string());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Set the group's avatar by the SHA-1 of an image already uploaded to Steam's avatar
    /// storage. (Uploading the image is done over Steam's web API, not the chat connection.)
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn set_avatar(&self, chat_group_id: u64, avatar_sha: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_SetChatRoomGroupAvatar_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_avatar_sha(avatar_sha.to_vec());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Show a broadcast in the group (`None` stops showing it).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn set_watching_broadcast(
        &self,
        chat_group_id: u64,
        broadcast: Option<(u32, u64)>,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_SetChatRoomGroupWatchingBroadcast_Request::new();
        request.set_chat_group_id(chat_group_id);
        let (account_id, channel_id) = broadcast.unwrap_or((0, 0));
        request.set_watching_broadcast_accountid(account_id);
        request.set_watching_broadcast_channel_id(channel_id);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Create a room (channel) in a group. Returns the new room's state, including its
    /// `chat_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn create_room(
        &self,
        chat_group_id: u64,
        name: &str,
        allow_voice: bool,
    ) -> Result<CChatRoomState, Box<dyn Error>> {
        let mut request = CChatRoom_CreateChatRoom_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_name(name.to_string());
        request.set_allow_voice(allow_voice);
        let mut response = self.connection.service_method(request).await?;
        Ok(response.chat_room.take().unwrap_or_default())
    }

    /// Rename a room.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn rename_room(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        name: &str,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_RenameChatRoom_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        request.set_name(name.to_string());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Move a room so it comes right after `move_after_chat_id` (`0` = first).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn reorder_room(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        move_after_chat_id: u64,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_ReorderChatRoom_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        request.set_move_after_chat_id(move_after_chat_id);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Delete a room.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn delete_room(&self, chat_group_id: u64, chat_id: u64) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_DeleteChatRoom_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Make a Steam group's (clan's) chat private (members only) or public. Returns the
    /// resulting privacy state.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn set_clan_chat_private(
        &self,
        clan_steam_id: SteamID,
        private: bool,
    ) -> Result<bool, Box<dyn Error>> {
        let mut request = CClanChatRooms_SetClanChatRoomPrivate_Request::new();
        request.set_steamid(clan_steam_id.into());
        request.set_chat_room_private(private);
        let response = self.connection.service_method(request).await?;
        Ok(response.chat_room_private())
    }
}
