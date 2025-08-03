use std::error::Error;
use std::env;
use SC_Sub_Poster::LogOn;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Environment Variables Example ===\n");

    // Get credentials from environment variables
    let account = env::var("STEAM_ACCOUNT").unwrap_or_else(|_| {
        println!("STEAM_ACCOUNT not set, using demo account");
        "anonymous".to_string()
    });
    
    let password = env::var("STEAM_PASSWORD").unwrap_or_else(|_| {
        println!("STEAM_PASSWORD not set, using empty password");
        "".to_string()
    });

    println!("Attempting login with account: {}", account);
    
    match LogOn::new(&account, &password).await {
        Ok(client) => {
            println!("✓ Login successful!");
            println!("Steam ID: {}", client.steam_id().steam3());
            
            // Get owned games
            match client.get_owned_games().await {
                Ok(games) => {
                    println!("✓ Found {} owned games:", games.len());
                    for (i, game) in games.iter().take(5).enumerate() {
                        println!("  {}. {}", i + 1, game);
                    }
                }
                Err(e) => {
                    println!("✗ Failed to get owned games: {:?}", e);
                }
            }
        }
        Err(e) => {
            println!("✗ Login failed: {:?}", e);
        }
    }

    Ok(())
} 