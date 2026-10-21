# SC Sub Poster (Kether.pl Steam Client Library)

A Rust library that wraps the `steam-vent` crate for logging in to Steam and using Steam Chat: group chats (messages, reactions, history, events, moderation, roles, invites, administration, voice signalling) and private messages between friends.

It started as a way to post `!sub` requests on our Steam Group Chat ;)

## Usage

Here's a basic example of how to use this library to login to Steam and send messages:

```rust
use std::{env, error::Error};
use SC_Sub_Poster::{ChatRoomClient, LogOn, SendGroupMessageParams};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create a client using credentials from environment variables
    let account = env::var("STEAM_ACCOUNT").expect("STEAM_ACCOUNT not set");
    let password = env::var("STEAM_PASSWORD").expect("STEAM_PASSWORD not set");
    
    // Login to Steam
    let logon = LogOn::new(&account, &password).await?;
    println!("✓ Login successful! Steam ID: {}", logon.steam_id().steam3());
    let snapshot = logon.session_snapshot();
    println!("Session cell id: {}", snapshot.cell_id);

    // Create chat room client
    let chat_client = ChatRoomClient::new(logon.connection().clone());
    
    // Get your chat rooms
    let chat_rooms = chat_client.groups().get_my_chat_rooms().await?;
    println!("Found {} chat rooms", chat_rooms.len());
    
    // Send a message to a specific group chat
    let group_id: u64 = 6887767; // Replace with your group ID
    let chat_id: u64 = 22190790;  // Replace with your chat ID
    let message = "Hello from Rust!";
    
    let response = chat_client
        .messaging()
        .send_group_message(
            SendGroupMessageParams::new(group_id, chat_id, message).with_echo_to_sender(true),
        )
        .await?;
    println!("✓ Message sent successfully!");
    println!("Modified message: {}", response.modified_message);
    
    Ok(())
}
```

### Environment Variables

Set these environment variables before running:

```bash
export STEAM_ACCOUNT="your_steam_username"
export STEAM_PASSWORD="your_steam_password"
export CHAT_GROUP_ID="your_group_id"  # Optional
export CHAT_ID="your_chat_id"         # Optional
```

### Features

- **Message Preprocessing**: Automatically processes BBCode formatting and mentions
- **Mention Support**: Handle `@all`, `@here`, and `[U:1:xxxxx]` SteamID mentions with serde-safe wrappers
- **Real-time Listening**: Listen for incoming friend and group messages with error-aware callbacks
- **Enhanced Messages**: Get detailed information about processed messages, including immutable session snapshots
- **Message Deletion**: Delete group chat messages by `(server_timestamp, ordinal)` or directly from `PreprocessedMessage` send responses
- **Message Reactions**: Add and remove emoticon or sticker reactions, list reactors, fetch reaction summaries from message history, and listen for real-time reaction events
- **Chat Groups**: List chat groups and their rooms via `get_my_chat_groups`, returning structured `ChatGroupInfo` with nested `ChatRoomInfo`
- **Tracing Spans**: Built-in `tracing` instrumentation for logon, chat dispatch, preprocessing, deletion, and reactions
- **Raw access**: every protobuf type in the public API is re-exported under `SC_Sub_Poster::proto`, and `notifications().listen_for_notification_with::<T, _>(...)` listens for any Steam notification type

### Cargo features

The original API (`groups()`, `messaging()`, `notifications()`) is always available. Everything else is opt-in:

| Feature | Handle | What it adds |
|---|---|---|
| `friend-messages` *(default)* | `client.friends()` | Private messages: `send_message`, `send_typing`, `ack_message` (mark read), `get_message_history`, `get_active_sessions`, `add_reaction` / `remove_reaction`, `get_friends_ui_status`; listeners for messages, typing / other events, and reactions |
| `chat-events` | `client.events()` | Group notifications: member joins/leaves/kicks/bans/mutes/rank & role changes, deleted messages, header changes, room list changes, your own membership changes, disconnects |
| `chat-moderation` | `client.moderation()` | `kick_user`, `ban_user` / `unban_user`, `get_ban_list`, `mute_user`, `get_invite_list`, `revoke_invite` |
| `chat-roles` | `client.roles()` | `get_roles`, `create_role`, `rename_role`, `reorder_role`, `delete_role`, `get_role_actions` / `replace_role_actions` (permissions), `add_role_to_user` / `remove_role_from_user`, `get_roles_for_user` |
| `chat-invites` | `client.invites()` | `invite_friend`, `create_invite_link`, `get_invite_links`, `delete_invite_link`, `get_invite_link_info`, `get_invite_info` (links are `https://s.team/chat/<code>`) |
| `chat-admin` | `client.admin()` | `create_group`, `save_group`, `rename_group`, `set_tagline`, `set_avatar`, `set_watching_broadcast`, `create_room` / `rename_room` / `reorder_room` / `delete_room`, `set_clan_chat_private` |
| `chat-voice` | `client.voice()` | `join` / `leave` a room's voice channel and listen for rejoin requests (signalling only, no audio) |
| `full` | | All of the above |

Always available as well: `groups().get_group_summary`, `get_clan_chat_group` (find a Steam group's chat by its SteamID), `set_session_active_groups`, `search_members`, `set_notification_preferences`, and `messaging().ack_chat_message` (mark a room as read).

Not covered: read-marker echoes from your other sessions (`steam-vent` 0.5 can't subscribe to them separately), mini-games, the virtualized member list for huge groups, and voice audio.

### Sending formatted messages

Steam does not render most raw BBCode sent by a client. The official client only ever sends mentions as BBCode; everything else is plain text that the server expands:

| What | Send | Helper (`preprocessing_helpers`) |
|---|---|---|
| Mention a user | `[mention=<account id>]@name[/mention]` | `mention(steam_id, name)`, `mention_account_id(id, name)` |
| Mention everyone / online | `[mention=all]@all[/mention]`, `[mention=here]@here[/mention]` | `mention_all()`, `mention_here()` |
| Emoticon | `:steamhappy:` | `emoticon(name)` |
| Code / pre / quote / spoiler / me | `/code text`, `/pre text`, `/quote text`, `/spoiler text`, `/me text` — the whole message | `slash_command(SlashCommand::Pre, text)` |
| Giphy / random / flip / store / sticker / room effect | `/giphy q`, `/random 1-100`, `/flip`, `/store app`, `/sticker name`, `/roomeffect name` | `slash_command(...)` |
| Links, images, clips | the plain URL | — |
| Untrusted text (names, user input) | escape `[` and `\` | `escape_bbcode(text)` |

The `BBCODE_TYPE_*` constants are for inspecting received messages, which contain the server-generated tags. (`format_with_bbcode`, `create_mention`, `create_all_mention`, `create_here_mention` and `chat_helpers::create_message_with_*` were removed in 0.10.0 because Steam doesn't render their output as intended.)

For more advanced usage, see the `examples/chat_demo.rs` file.

## Error Handling

- `LogOn::new` and `LogOn::new_anonymous` return a boxed `LogonError` that can be downcast for retry hints (`ErrorInventoryEntry`).
- Notification loops expose `listen_for_*_messages_with` helpers that bubble transport failures rather than silently retrying forever.

## Dependencies

This library depends on:
- `steam-vent` - Steam network interaction
- `tokio` - Async runtime
- `tracing` - Logging
- `serde` - Serialization

## License

LGPL-3.0-only

However, the author requests in good faith that it not be used for military, surveillance, or malicious purposes.
