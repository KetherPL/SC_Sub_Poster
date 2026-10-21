// SPDX-License-Identifier: LGPL-3.0-only

//! Group chat invites: inviting friends, invite links (`https://s.team/chat/<code>`) and
//! resolving invite codes.

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_CreateInviteLink_Request, CChatRoom_DeleteInviteLink_Request,
    CChatRoom_GetInviteInfo_Request, CChatRoom_GetInviteInfo_Response,
    CChatRoom_GetInviteLinkInfo_Request, CChatRoom_GetInviteLinkInfo_Response,
    CChatRoom_GetInviteLinksForGroup_Request, CChatRoom_InviteFriendToChatRoomGroup_Request,
};
use steamid_ng::SteamID;

use crate::chatroom::ChatRoomClient;

/// Base URL of Steam chat invite links; append the invite code.
pub const INVITE_LINK_BASE_URL: &str = "https://s.team/chat/";

/// Extracts the invite code from an invite link (`https://s.team/chat/<code>`) or returns
/// the input unchanged if it already is a bare code.
pub fn invite_code_from_link(link_or_code: &str) -> &str {
    let trimmed = link_or_code.trim().trim_end_matches('/');
    trimmed
        .strip_prefix(INVITE_LINK_BASE_URL)
        .or_else(|| trimmed.strip_prefix("http://s.team/chat/"))
        .unwrap_or(trimmed)
}

/// A newly created invite link.
#[derive(Debug, Clone)]
pub struct CreatedInviteLink {
    /// The invite code.
    pub invite_code: String,
    /// The full invite URL.
    pub url: String,
    /// How long the link is valid, in seconds (`0` = never expires).
    pub seconds_valid: u32,
}

/// An existing invite link of a group.
#[derive(Debug, Clone)]
pub struct InviteLink {
    /// The invite code.
    pub invite_code: String,
    /// The full invite URL.
    pub url: String,
    /// Who created the link.
    pub creator: SteamID,
    /// When the link expires (Unix time; `0` = never).
    pub time_expires: u32,
    /// The room the link leads to (`0` = the group's default room).
    pub chat_id: u64,
}

/// Invite operations. Get one with [`ChatRoomClient::invites`].
pub struct ChatInvites<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access group chat invites and invite links.
    pub fn invites(&self) -> ChatInvites<'_> {
        ChatInvites {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatInvites<'a> {
    /// Invite a friend to a group, optionally pointing them at a specific room.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn invite_friend(
        &self,
        chat_group_id: u64,
        friend: SteamID,
        chat_id: Option<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_InviteFriendToChatRoomGroup_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(friend.into());
        if let Some(chat_id) = chat_id {
            request.set_chat_id(chat_id);
        }
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Create an invite link. `seconds_valid` of `None` (or `0`) creates a link that never
    /// expires; `chat_id` points it at a specific room.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn create_invite_link(
        &self,
        chat_group_id: u64,
        seconds_valid: Option<u32>,
        chat_id: Option<u64>,
    ) -> Result<CreatedInviteLink, Box<dyn Error>> {
        let mut request = CChatRoom_CreateInviteLink_Request::new();
        request.set_chat_group_id(chat_group_id);
        if let Some(seconds_valid) = seconds_valid {
            request.set_seconds_valid(seconds_valid);
        }
        if let Some(chat_id) = chat_id {
            request.set_chat_id(chat_id);
        }
        let response = self.connection.service_method(request).await?;
        Ok(CreatedInviteLink {
            url: format!("{}{}", INVITE_LINK_BASE_URL, response.invite_code()),
            invite_code: response.invite_code().to_string(),
            seconds_valid: response.seconds_valid(),
        })
    }

    /// List a group's invite links.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_invite_links(
        &self,
        chat_group_id: u64,
    ) -> Result<Vec<InviteLink>, Box<dyn Error>> {
        let mut request = CChatRoom_GetInviteLinksForGroup_Request::new();
        request.set_chat_group_id(chat_group_id);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .invite_links
            .iter()
            .map(|link| InviteLink {
                url: format!("{}{}", INVITE_LINK_BASE_URL, link.invite_code()),
                invite_code: link.invite_code().to_string(),
                creator: SteamID::from(link.steamid_creator()),
                time_expires: link.time_expires(),
                chat_id: link.chat_id(),
            })
            .collect())
    }

    /// Delete an invite link (accepts a bare code or a full link).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn delete_invite_link(
        &self,
        chat_group_id: u64,
        link_or_code: &str,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_DeleteInviteLink_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_invite_code(invite_code_from_link(link_or_code).to_string());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Look up an invite link (bare code or full link): which group it leads to, who made it,
    /// when it expires, and whether you are kicked/banned there. Join the group afterwards
    /// with `join_chat_room(chat_group_id, Some(code))`.
    ///
    /// # Errors
    ///
    /// Returns an error if the code is invalid or the request fails.
    pub async fn get_invite_link_info(
        &self,
        link_or_code: &str,
    ) -> Result<CChatRoom_GetInviteLinkInfo_Response, Box<dyn Error>> {
        let mut request = CChatRoom_GetInviteLinkInfo_Request::new();
        request.set_invite_code(invite_code_from_link(link_or_code).to_string());
        Ok(self.connection.service_method(request).await?)
    }

    /// Look up an invite you received (by group/room or invite code).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_invite_info(
        &self,
        invitee: SteamID,
        chat_group_id: u64,
        chat_id: Option<u64>,
        invite_code: Option<&str>,
    ) -> Result<CChatRoom_GetInviteInfo_Response, Box<dyn Error>> {
        let mut request = CChatRoom_GetInviteInfo_Request::new();
        request.set_steamid_invitee(invitee.into());
        request.set_chat_group_id(chat_group_id);
        if let Some(chat_id) = chat_id {
            request.set_chat_id(chat_id);
        }
        if let Some(code) = invite_code {
            request.set_invite_code(invite_code_from_link(code).to_string());
        }
        Ok(self.connection.service_method(request).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::invite_code_from_link;

    #[test]
    fn invite_code_from_link_accepts_links_and_codes() {
        assert_eq!(invite_code_from_link("https://s.team/chat/AbCd1234"), "AbCd1234");
        assert_eq!(invite_code_from_link(" https://s.team/chat/AbCd1234/ "), "AbCd1234");
        assert_eq!(invite_code_from_link("http://s.team/chat/AbCd1234"), "AbCd1234");
        assert_eq!(invite_code_from_link("AbCd1234"), "AbCd1234");
    }
}
