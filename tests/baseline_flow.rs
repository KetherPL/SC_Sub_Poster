// SPDX-License-Identifier: LGPL-3.0-only

use SC_Sub_Poster::{chatroom::ChatRoomClient, LogOn};

/// Smoke-test for anonymous connections.
///
/// This mirrors the behaviour of `examples/main_demo.rs`, but is marked as ignored because it
/// depends on live Steam infrastructure.
#[tokio::test]
#[ignore = "Requires network access to Steam"]
async fn anonymous_connection_smoke() {
    let client = LogOn::new_anonymous()
        .await
        .expect("anonymous connection should succeed");

    let steam_id = client.steam_id();
    assert_ne!(steam_id.account_id(), 0, "SteamID should be non-zero");
}

/// Characterisation test for fetching chat rooms over an anonymous connection.
#[tokio::test]
#[ignore = "Requires network access to Steam"]
async fn chat_room_listing_smoke() {
    let client = LogOn::new_anonymous()
        .await
        .expect("anonymous connection should succeed");

    let chat_client = ChatRoomClient::new(client.connection().clone());
    chat_client
        .get_my_chat_rooms()
        .await
        .expect("listing chat rooms should succeed");
}

