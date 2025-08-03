# SC Sub Poster (Kether.pl Steam Client Library)

A Rust library that provides a wrapper around the `steam-vent` crate for logging in to Steam and operating on the Steam Group Chats.

Actually all we need just to post `!sub` requests on our Steam Group Chat ;)

## Usage

Here's a basic example of how to use this library to login to Steam and send messages:

```rust
use std::{env, error::Error};
use SC_Sub_Poster::{LogOn, ChatRoomClient};

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

    // Create chat room client
    let chat_client = ChatRoomClient::new(logon.connection().clone());
    
    // Get your chat rooms
    let chat_rooms = chat_client.get_my_chat_rooms().await?;
    println!("Found {} chat rooms", chat_rooms.len());
    
    // Send a message to a specific group chat
    let group_id: u64 = 6887767; // Replace with your group ID
    let chat_id: u64 = 22190790;  // Replace with your chat ID
    let message = "Hello from Rust!";
    
    let response = chat_client.send_group_message(group_id, chat_id, message, true).await?;
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
- **Mention Support**: Handle `@all`, `@here`, and `@steamid` mentions
- **Real-time Listening**: Listen for incoming friend and group messages
- **Enhanced Messages**: Get detailed information about processed messages

For more advanced usage, see the `examples/chat_demo.rs` file.

## Dependencies

This library depends on:
- `steam-vent` - Steam network interaction
- `tokio` - Async runtime
- `tracing` - Logging
- `serde` - Serialization

## License

LGPL-3.0-only
