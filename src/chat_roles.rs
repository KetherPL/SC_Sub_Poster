// SPDX-License-Identifier: LGPL-3.0-only

//! Group chat roles and their permissions ("role actions").
//!
//! Permissions are the raw [`CChatRoleActions`](crate::proto::chat::CChatRoleActions) protobuf message (re-exported in
//! [`crate::proto::chat`]); set its `can_*` flags and pass it to
//! [`ChatRoles::replace_role_actions`](crate::chat_roles::ChatRoles::replace_role_actions).

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoleActions, CChatRoom_AddRoleToUser_Request, CChatRoom_CreateRole_Request,
    CChatRoom_DeleteRoleFromUser_Request, CChatRoom_DeleteRole_Request,
    CChatRoom_GetRoleActions_Request, CChatRoom_GetRolesForUser_Request,
    CChatRoom_GetRoles_Request, CChatRoom_RenameRole_Request, CChatRoom_ReorderRole_Request,
    CChatRoom_ReplaceRoleActions_Request,
};
use steamid_ng::SteamID;

use crate::chatroom::ChatRoomClient;

/// A role in a chat group.
#[derive(Debug, Clone)]
pub struct ChatRole {
    /// The role's id.
    pub role_id: u64,
    /// The role's display name.
    pub name: String,
    /// Sort position.
    pub ordinal: u32,
}

/// Role operations. Get one with [`ChatRoomClient::roles`].
pub struct ChatRoles<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access group chat roles and permissions.
    pub fn roles(&self) -> ChatRoles<'_> {
        ChatRoles {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatRoles<'a> {
    /// List the group's roles.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_roles(&self, chat_group_id: u64) -> Result<Vec<ChatRole>, Box<dyn Error>> {
        let mut request = CChatRoom_GetRoles_Request::new();
        request.set_chat_group_id(chat_group_id);
        let response = self.connection.service_method(request).await?;
        Ok(response
            .roles
            .iter()
            .map(|role| ChatRole {
                role_id: role.role_id(),
                name: role.name().to_string(),
                ordinal: role.ordinal(),
            })
            .collect())
    }

    /// Create a role. Returns its initial permissions (including the new `role_id`).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn create_role(
        &self,
        chat_group_id: u64,
        name: &str,
    ) -> Result<CChatRoleActions, Box<dyn Error>> {
        let mut request = CChatRoom_CreateRole_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_name(name.to_string());
        let mut response = self.connection.service_method(request).await?;
        Ok(response.actions.take().unwrap_or_default())
    }

    /// Rename a role.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn rename_role(
        &self,
        chat_group_id: u64,
        role_id: u64,
        name: &str,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_RenameRole_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        request.set_name(name.to_string());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Move a role to sort position `ordinal`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn reorder_role(
        &self,
        chat_group_id: u64,
        role_id: u64,
        ordinal: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_ReorderRole_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        request.set_ordinal(ordinal);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Delete a role.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn delete_role(&self, chat_group_id: u64, role_id: u64) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_DeleteRole_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Get permissions of one role, or of every role when `role_id` is `None`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_role_actions(
        &self,
        chat_group_id: u64,
        role_id: Option<u64>,
    ) -> Result<Vec<CChatRoleActions>, Box<dyn Error>> {
        let mut request = CChatRoom_GetRoleActions_Request::new();
        request.set_chat_group_id(chat_group_id);
        if let Some(role_id) = role_id {
            request.set_role_id(role_id);
        }
        let response = self.connection.service_method(request).await?;
        Ok(response.actions)
    }

    /// Replace a role's permissions with `actions`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn replace_role_actions(
        &self,
        chat_group_id: u64,
        role_id: u64,
        actions: CChatRoleActions,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_ReplaceRoleActions_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        request.actions = Some(actions).into();
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Give a user a role.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn add_role_to_user(
        &self,
        chat_group_id: u64,
        role_id: u64,
        user: SteamID,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_AddRoleToUser_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        request.set_steamid(user.into());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Take a role away from a user.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or you lack the permission.
    pub async fn remove_role_from_user(
        &self,
        chat_group_id: u64,
        role_id: u64,
        user: SteamID,
    ) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_DeleteRoleFromUser_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_role_id(role_id);
        request.set_steamid(user.into());
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// List the role ids a user has.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_roles_for_user(
        &self,
        chat_group_id: u64,
        user: SteamID,
    ) -> Result<Vec<u64>, Box<dyn Error>> {
        let mut request = CChatRoom_GetRolesForUser_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_steamid(user.into());
        let response = self.connection.service_method(request).await?;
        Ok(response.role_ids)
    }
}
