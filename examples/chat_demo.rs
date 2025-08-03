use std::{env, error::Error};
use std::time::Duration;
use tokio::time::sleep;
use SC_Sub_Poster::{
    LogOn, ChatRoomClient, chat_helpers, preprocessing_helpers,
    FriendMessage, GroupChatMessage, EnhancedGroupChatMessage,
    MessagePreprocessor, PreprocessedMessage,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Steam Chat Room Demo with Preprocessing ===\n");

    // Create a client (using credentials from environment variables)
    let account = env::var("STEAM_ACCOUNT").unwrap_or_else(|_| {
        println!("STEAM_ACCOUNT not set, using demo account");
        "anonymous".to_string()
    });
    
    let password = env::var("STEAM_PASSWORD").unwrap_or_else(|_| {
        println!("STEAM_PASSWORD not set, using empty password");
        "".to_string()
    });
    
    let logon = match LogOn::new(&account, &password).await {
        Ok(client) => {
            println!("✓ Login successful!");
            println!("Steam ID: {}", client.steam_id().steam3());
            client
        }
        Err(e) => {
            println!("✗ Login failed: {:?}", e);
            println!("Falling back to anonymous connection for demo...");
            LogOn::new_anonymous().await?
        }
    };

    // Create chat room client
    let chat_client = ChatRoomClient::new(logon.connection().clone());
    
    println!("\n1. Getting chat rooms...");
    match chat_client.get_my_chat_rooms().await {
        Ok(chat_rooms) => {
            println!("✓ Found {} chat rooms:", chat_rooms.len());
            for (i, room) in chat_rooms.iter().enumerate() {
                println!("  {}. {} (Group: {})", i + 1, room.chat_name, room.chat_group_name);
                println!("     Group ID: {}, Chat ID: {}", room.chat_group_id, room.chat_id);
            }
        }
        Err(e) => {
            println!("✗ Failed to get chat rooms: {:?}", e);
        }
    }

    println!("\n2. Testing message preprocessing...");
    
    // Test different types of messages
    let test_messages = vec![
        "Hello world!",
        "Hello @all users!",
        "Hello @here users!",
        "Check out this [b]bold[/b] text!",
        "Here's some [i]italic[/i] and [u]underlined[/u] text",
        "Code example: [code]println!(\"Hello, world!\");[/code]",
        "Spoiler alert: [spoiler]This is hidden[/spoiler]",
        "Visit [url=https://steamcommunity.com]Steam Community[/url]",
    ];

    for (i, message) in test_messages.iter().enumerate() {
        println!("  {}. Testing: \"{}\"", i + 1, message);
        let preprocessed = MessagePreprocessor::preprocess_message(message);
        
        println!("     Original: {}", preprocessed.original_message);
        println!("     Modified: {}", preprocessed.modified_message);
        
        if let Some(mentions) = &preprocessed.mentions {
            println!("     Mentions: @all={}, @here={}, SteamIDs={}", 
                mentions.mention_all, mentions.mention_here, mentions.mention_steamids.len());
        }
        
        println!("     BBCode nodes: {}", preprocessed.message_bbcode_parsed.len());
    }

    println!("\n3. Testing Steam ID parsing and mentions...");
    let steam_id_str = "[U:1:1531059355]";
    match chat_helpers::parse_steam_id(steam_id_str) {
        Ok(steam_id) => {
            println!("✓ Successfully parsed Steam ID: {}", steam_id.steam3());
            let formatted = chat_helpers::format_steam_id(steam_id);
            println!("  Formatted: {}", formatted);
            
            // Test mention creation
            let mention = preprocessing_helpers::create_mention(steam_id);
            println!("  Mention: {}", mention);
            
            let message_with_mention = chat_helpers::create_message_with_mentions("Hello", &[steam_id]);
            println!("  Message with mention: {}", message_with_mention);
        }
        Err(e) => {
            println!("✗ Failed to parse Steam ID: {:?}", e);
        }
    }

    println!("\n4. Testing group message sending with preprocessing...");
    // Get chat group ID and chat ID from environment variables
    let chat_group_id = std::env::var("CHAT_GROUP_ID").unwrap_or_else(|_| {
        println!("CHAT_GROUP_ID not set, using default from chat rooms");
        "6887767".to_string() // Default to Kether.pl group ID
    });
    
    let chat_id = std::env::var("CHAT_ID").unwrap_or_else(|_| {
        println!("CHAT_ID not set, using default from chat rooms");
        "22190790".to_string() // Default to Kether.pl chat ID
    });

    println!("Using Chat Group ID: {}, Chat ID: {}", chat_group_id, chat_id);
    
    // Parse the IDs
    let group_id: u64 = chat_group_id.parse().expect("Invalid CHAT_GROUP_ID");
    let chat_id: u64 = chat_id.parse().expect("Invalid CHAT_ID");
    
    // Send test message with preprocessing
    let test_message = "test [mention=here]@online[/mention]";
    println!("Sending message: \"{}\"", test_message);
    
    match chat_client.send_group_message(group_id, chat_id, test_message, true).await {
        Ok(preprocessed_response) => {
            println!("✓ Group message sent successfully!");
            println!("  Original message: {}", preprocessed_response.original_message);
            println!("  Modified message: {}", preprocessed_response.modified_message);
            println!("  Server timestamp: {}", preprocessed_response.server_timestamp.unwrap_or(0));
            println!("  Ordinal: {}", preprocessed_response.ordinal.unwrap_or(0));
            
            if let Some(mentions) = &preprocessed_response.mentions {
                println!("  Mentions detected: @all={}, @here={}", 
                    mentions.mention_all, mentions.mention_here);
            }
            
            println!("  BBCode nodes: {}", preprocessed_response.message_bbcode_parsed.len());
        }
        Err(e) => {
            println!("✗ Failed to send group message: {:?}", e);
        }
    }

    println!("\n5. Setting up enhanced message listeners...");
    
    // Spawn a task to listen for friend messages
    let friend_chat_client = ChatRoomClient::new(logon.connection().clone());
    tokio::spawn(async move {
        println!("  Listening for friend messages...");
        if let Err(e) = friend_chat_client.listen_for_friend_messages(|msg: FriendMessage| {
            println!("📨 Friend Message from {}: {}", 
                chat_helpers::format_steam_id(msg.steam_id), 
                msg.message);
        }).await {
            println!("  Friend message listener error: {:?}", e);
        }
    });

    // Spawn a task to listen for enhanced group messages
    let group_chat_client = ChatRoomClient::new(logon.connection().clone());
    tokio::spawn(async move {
        println!("  Listening for enhanced group messages...");
        if let Err(e) = group_chat_client.listen_for_group_messages(|msg: EnhancedGroupChatMessage| {
            println!("💬 Enhanced Group Message in {} from {}: {}", 
                msg.chat_name,
                chat_helpers::format_steam_id(msg.sender_steam_id), 
                msg.message);
            
            // Show preprocessing information
            if let Some(mentions) = &msg.preprocessed.mentions {
                println!("    Mentions: @all={}, @here={}", 
                    mentions.mention_all, mentions.mention_here);
            }
            println!("    BBCode nodes: {}", msg.preprocessed.message_bbcode_parsed.len());
        }).await {
            println!("  Enhanced group message listener error: {:?}", e);
        }
    });

    println!("\n6. Demo completed!");
    println!("The enhanced message listeners are now running in the background.");
    println!("Features demonstrated:");
    println!("  ✓ Message preprocessing with BBCode parsing");
    println!("  ✓ Mention detection (@all, @here, @steamid)");
    println!("  ✓ Enhanced message responses with preprocessing data");
    println!("  ✓ Real-time message listening with preprocessing");
    
    // Keep the program running for a bit to show the listeners
    println!("\nWaiting 5 seconds to demonstrate listeners...");
    sleep(Duration::from_secs(5)).await;
    
    println!("Demo finished!");
    Ok(())
} 