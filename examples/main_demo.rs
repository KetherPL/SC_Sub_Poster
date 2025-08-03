use std::error::Error;
use SC_Sub_Poster::LogOn;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Steam Client Demo ===\n");

    // Test anonymous connection first
    println!("1. Testing anonymous connection...");
    match LogOn::new_anonymous().await {
        Ok(client) => {
            println!("✓ Anonymous connection successful!");
            println!("  Steam ID: {}", client.steam_id().steam3());
            
            // Test connection
            match client.test_connection().await {
                Ok(_) => println!("✓ Connection test successful!"),
                Err(e) => println!("⚠ Connection test failed (this is normal for anonymous): {:?}", e),
            }
        }
        Err(e) => {
            println!("✗ Anonymous connection failed: {:?}", e);
        }
    }

    // Test authenticated login if credentials are provided
    println!("\n2. Testing authenticated login...");
    
    // Get credentials from environment variables
    let account = std::env::var("STEAM_ACCOUNT").unwrap_or_else(|_| {
        println!("STEAM_ACCOUNT not set, skipping authenticated login");
        return String::new();
    });
    
    let password = std::env::var("STEAM_PASSWORD").unwrap_or_else(|_| {
        println!("STEAM_PASSWORD not set, skipping authenticated login");
        return String::new();
    });

    if !account.is_empty() && !password.is_empty() {
        match LogOn::new(&account, &password).await {
            Ok(client) => {
                println!("✓ Authenticated login successful!");
                println!("  Steam ID: {}", client.steam_id().steam3());
                
                // Get owned games
                println!("\n3. Fetching owned games...");
                match client.get_owned_games().await {
                    Ok(games) => {
                        println!("✓ Found {} owned games:", games.len());
                        for (i, game) in games.iter().take(10).enumerate() {
                            println!("  {}. {}", i + 1, game);
                        }
                        if games.len() > 10 {
                            println!("  ... and {} more games", games.len() - 10);
                        }
                    }
                    Err(e) => {
                        println!("✗ Failed to get owned games: {:?}", e);
                    }
                }
            }
            Err(e) => {
                println!("✗ Authenticated login failed: {:?}", e);
                println!("  This might be due to:");
                println!("  - Invalid credentials");
                println!("  - Two-factor authentication required");
                println!("  - Steam Guard enabled");
                println!("  - Account restrictions");
            }
        }
    } else {
        println!("  Skipping authenticated login (no credentials provided)");
        println!("  Set STEAM_ACCOUNT and STEAM_PASSWORD environment variables to test authenticated login");
    }

    println!("\n=== Demo completed ===");
    println!("To test authenticated features, run:");
    println!("  export STEAM_ACCOUNT=your_username");
    println!("  export STEAM_PASSWORD=your_password");
    println!("  cargo run --example main_demo");
    
    Ok(())
} 