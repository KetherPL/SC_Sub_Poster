// SPDX-License-Identifier: LGPL-3.0-only

//! Voice chat signalling for group chat rooms.
//!
//! This only tells Steam you joined or left a room's voice channel; it does **not** carry
//! audio (Steam voice uses a separate WebRTC transport that this crate does not implement).

use std::error::Error;

use steam_vent::ConnectionTrait;
use steam_vent_proto::steammessages_chat_steamclient::{
    CChatRoom_JoinVoiceChat_Request, CChatRoom_LeaveVoiceChat_Request,
    CChatRoom_NotifyShouldRejoinChatRoomVoiceChat_Notification,
};

use crate::chatroom::{CallbackResult, ChatRoomClient, listen_notifications};

/// Voice signalling operations. Get one with [`ChatRoomClient::voice`].
pub struct ChatVoice<'a> {
    connection: &'a steam_vent::Connection,
}

impl ChatRoomClient {
    /// Access voice chat signalling (join/leave only, no audio).
    pub fn voice(&self) -> ChatVoice<'_> {
        ChatVoice {
            connection: self.connection(),
        }
    }
}

impl<'a> ChatVoice<'a> {
    /// Join a room's voice channel. Returns the voice chat id.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the room has no voice.
    pub async fn join(&self, chat_group_id: u64, chat_id: u64) -> Result<u64, Box<dyn Error>> {
        let mut request = CChatRoom_JoinVoiceChat_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        let response = self.connection.service_method(request).await?;
        Ok(response.voice_chatid())
    }

    /// Leave a room's voice channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn leave(&self, chat_group_id: u64, chat_id: u64) -> Result<(), Box<dyn Error>> {
        let mut request = CChatRoom_LeaveVoiceChat_Request::new();
        request.set_chat_group_id(chat_group_id);
        request.set_chat_id(chat_id);
        self.connection.service_method(request).await?;
        Ok(())
    }

    /// Listen for Steam asking you to rejoin a voice channel (e.g. after reconnecting).
    /// The callback receives `(chat_group_id, chat_id)`; return `Err` to stop listening.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification stream fails or the callback returns an error.
    pub async fn listen_for_rejoin_requests_with<F>(&self, mut callback: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(u64, u64) -> CallbackResult + Send + 'static,
    {
        listen_notifications(
            self.connection,
            move |notification: CChatRoom_NotifyShouldRejoinChatRoomVoiceChat_Notification| {
                callback(notification.chat_group_id(), notification.chat_id())
            },
        )
        .await
    }
}
