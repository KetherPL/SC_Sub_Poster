// SPDX-License-Identifier: LGPL-3.0-only

//! Internal plumbing for Steam service methods that have no response.
//!
//! Steam marks some client-to-server service methods as "notifications" (their generated
//! `RpcMethod::Response` is `()`), e.g. `ChatRoom.AckChatMessage#1` or
//! `FriendMessages.AckMessage#1`. The server never answers them, so
//! `ConnectionTrait::service_method` would wait until it times out. `steam-vent` keeps its own
//! wrapper for these crate-private, so this module provides an equivalent one that is sent
//! without a job id and returns as soon as the message is written.

use std::fmt::Debug;
use std::io::Write;

use steam_vent::message::{EncodableMessage, NetMessage};
use steam_vent::{ConnectionTrait, NetMessageHeader, NetworkError, ServiceMethodRequest};
use steam_vent_proto::enums_clientserver::EMsg;

/// A service method call that is sent without waiting for (or expecting) a response.
#[derive(Debug)]
pub(crate) struct FireAndForget<R>(pub R);

impl<R: ServiceMethodRequest + Send> EncodableMessage for FireAndForget<R> {
    fn write_body<W: Write>(&self, mut writer: W) -> Result<(), std::io::Error> {
        ServiceMethodRequest::write(&self.0, &mut writer)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))
    }

    fn encode_size(&self) -> usize {
        ServiceMethodRequest::encode_size(&self.0)
    }

    fn process_header(&self, header: &mut NetMessageHeader) {
        header.target_job_name = Some(R::REQ_NAME.into());
    }
}

impl<R: ServiceMethodRequest + Send> NetMessage for FireAndForget<R> {
    type KindEnum = EMsg;
    const KIND: Self::KindEnum = EMsg::k_EMsgServiceMethodCallFromClient;
    const IS_PROTOBUF: bool = true;
}

/// Sends a no-response service method (a client-side "notification") to Steam.
pub(crate) async fn notify<R>(
    connection: &steam_vent::Connection,
    request: R,
) -> Result<(), NetworkError>
where
    R: ServiceMethodRequest + Send,
{
    connection.send(FireAndForget(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_vent_proto::steammessages_chat_steamclient::CChatRoom_AckChatMessage_Notification;

    #[test]
    fn fire_and_forget_sets_method_name_and_encodes_body() {
        let mut request = CChatRoom_AckChatMessage_Notification::new();
        request.set_chat_group_id(1);
        request.set_chat_id(2);
        request.set_timestamp(3);
        let message = FireAndForget(request);

        let mut header = NetMessageHeader::default();
        message.process_header(&mut header);
        assert_eq!(
            header.target_job_name.as_deref(),
            Some("ChatRoom.AckChatMessage#1")
        );

        let mut body = Vec::new();
        message.write_body(&mut body).expect("encode");
        assert_eq!(body.len(), message.encode_size());
        assert!(!body.is_empty());
    }
}
