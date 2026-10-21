// SPDX-License-Identifier: LGPL-3.0-only

//! Always-available group chat operations added on top of the original API: group summaries,
//! finding a Steam group's chat, session activation, member search, notification
//! preferences and read markers.

use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_AckChatMessage_Notification, CChatRoom_GetChatRoomGroupSummary_Request,
    CChatRoom_GetChatRoomGroupSummary_Response, CChatRoom_SearchMembers_Request,
    CChatRoom_SetSessionActiveChatRoomGroups_Request,
    CChatRoom_SetSessionActiveChatRoomGroups_Response,
    CChatRoom_SetUserChatGroupPreferences_Request, CClanChatRooms_GetClanChatRoomInfo_Request,
};
use steamid_ng::SteamID;

use crate::chatroom::{ChatRoomGroups, ChatRoomMessaging, steam_id_from_account_id};
use crate::rpc;

/// A member found by [`ChatRoomGroups::search_members`].
#[derive(Debug, Clone)]
pub struct ChatMemberMatch {
    /// The member.
    pub steam_id: SteamID,
    /// Their current persona (display) name.
    pub persona_name: String,
}

static NEXT_SEARCH_ID: AtomicU64 = AtomicU64::new(1);

impl<'a> ChatRoomGroups<'a> {
    /// Get a group's summary: name, tagline, owner, rooms, member counts, your rank and
    /// roles.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_group_summary(
        &self,
        chat_group_id: u64,
    ) -> Result<CChatRoom_GetChatRoomGroupSummary_Response, Box<dyn Error>> {
        let mut request = CChatRoom_GetChatRoomGroupSummary_Request::new();
        request.set_chat_group_id(chat_group_id);
        Ok(self.connection.service_method(request).await?)
    }

    /// Find the chat group of a Steam group (clan), e.g. to get its `chat_group_id` from the
    /// group's SteamID. With `autocreate`, Steam creates the chat if it does not exist yet.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_clan_chat_group(
        &self,
        clan_steam_id: SteamID,
        autocreate: bool,
    ) -> Result<CChatRoom_GetChatRoomGroupSummary_Response, Box<dyn Error>> {
        let mut request = CClanChatRooms_GetClanChatRoomInfo_Request::new();
        request.set_steamid(clan_steam_id.into());
        request.set_autocreate(autocreate);
        let mut response = self.connection.service_method(request).await?;
        Ok(response.chat_group_summary.take().unwrap_or_default())
    }

    /// Tell Steam which groups this session is actively viewing. Steam then sends full state
    /// (members, rooms) for them and keeps delivering their notifications. Returns the
    /// groups' current state.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn set_session_active_groups(
        &self,
        chat_group_ids: &[u64],
    ) -> Result<CChatRoom_SetSessionActiveChatRoomGroups_Response, Box<dyn Error>> {
        let mut request = CChatRoom_SetSessionActiveChatRoomGroups_Request::new();
        request.chat_group_ids = chat_group_ids.to_vec();
        request.chat_groups_data_requested = chat_group_ids.to_vec();
        Ok(self.connection.service_method(request).await?)
    }

    /// Search a group's members by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn search_members(
        &self,
        chat_group_id: u64,
        search_text: &str,
        max_results: u32,
    ) -> Result<Vec<ChatMemberMatch>, Box<dyn Error>> {
        let mut request = CChatRoom_SearchMembers_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_search_id(NEXT_SEARCH_ID.fetch_add(1, Ordering::Relaxed));
        request.set_search_text(search_text.to_string());
        request.set_max_results(max_results.min(i32::MAX as u32) as i32);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .matching_members
            .iter()
            .map(|member| ChatMemberMatch {
                steam_id: steam_id_from_account_id(member.accountid() as u32),
                persona_name: member.persona.player_name().to_string(),
            })
            .collect())
    }

    /// Set your notification preferences for a group and its rooms. Build the request with
    /// the raw protobuf type ([`crate::proto::chat`]), setting `chat_group_id` and the
    /// group/room preference fields you want to change.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn set_notification_preferences(
        &self,
        request: CChatRoom_SetUserChatGroupPreferences_Request,
    ) -> Result<(), Box<dyn Error>> {
        self.connection.service_method(request).await?;
        Ok(())
    }
}

impl<'a> ChatRoomMessaging<'a> {
    /// Mark a room as read up to `timestamp` (use the latest message's server timestamp).
    /// Steam sends no response to this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the message could not be sent.
    pub async fn ack_chat_message(
        &self,
        chat_group_id: u64,
        chat_id: u64,
        timestamp: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_AckChatMessage_Notification::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        request.set_timestamp(timestamp);
        rpc::notify(self.connection, request).await?;
        Ok(())
    }
}
