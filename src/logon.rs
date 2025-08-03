// SPDX-License-Identifier: LGPL-3.0-only

use std::error::Error;
use steam_vent::auth::{
    AuthConfirmationHandler, ConsoleAuthConfirmationHandler, DeviceConfirmationHandler,
    FileGuardDataStore,
};
use steam_vent::{Connection, ConnectionTrait, ServerList};
use steamid_ng::SteamID;

/// Steam client wrapper for authenticated and anonymous operations
pub struct KetherSteamClient {
    connection: Connection,
}

impl KetherSteamClient {
    /// Create a new Steam client with provided credentials
    pub async fn new(account: &str, password: &str) -> Result<Self, Box<dyn Error>> {
        let server_list = ServerList::discover().await?;
        let connection = Connection::login(
            &server_list,
            account,
            password,
            FileGuardDataStore::user_cache(),
            ConsoleAuthConfirmationHandler::default().or(DeviceConfirmationHandler),
        )
        .await?;

        Ok(Self { connection })
    }

    /// Create an anonymous Steam client for testing
    pub async fn new_anonymous() -> Result<Self, Box<dyn Error>> {
        let server_list = ServerList::discover().await?;
        let connection = Connection::anonymous(&server_list).await?;

        Ok(Self { connection })
    }

    /// Get the Steam ID of the connected user
    pub fn steam_id(&self) -> SteamID {
        self.connection.steam_id()
    }

    /// Get the connection for direct access to Steam services
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Get a mutable reference to the connection
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Test if the connection is working by requesting server list
    pub async fn test_connection(&self) -> Result<(), Box<dyn Error>> {
        use steam_vent::proto::steammessages_gameservers_steamclient::CGameServers_GetServerList_Request;

        let mut req = CGameServers_GetServerList_Request::new();
        req.set_limit(5);
        req.set_filter(r"\appid\440".into()); // TF2 servers for testing
        
        let servers = self.connection.service_method(req).await?;
        
        if servers.servers.is_empty() {
            return Err("No servers found, but connection is working".into());
        }
        
        Ok(())
    }

    /// Get owned games for the logged-in user
    pub async fn get_owned_games(&self) -> Result<Vec<GameInfo>, Box<dyn Error>> {
        use steam_vent::proto::steammessages_player_steamclient::CPlayer_GetOwnedGames_Request;

        let req = CPlayer_GetOwnedGames_Request {
            steamid: Some(self.connection.steam_id().into()),
            include_appinfo: Some(true),
            include_played_free_games: Some(true),
            ..CPlayer_GetOwnedGames_Request::default()
        };

        let games = self.connection.service_method(req).await?;
        
        let game_info: Vec<GameInfo> = games
            .games
            .into_iter()
            .map(|game| GameInfo {
                app_id: game.appid() as u32,
                name: game.name().to_string(),
                playtime_forever: game.playtime_forever() as u32,
            })
            .collect();

        Ok(game_info)
    }
}

/// Information about a Steam game
#[derive(Debug, Clone)]
pub struct GameInfo {
    pub app_id: u32,
    pub name: String,
    pub playtime_forever: u32,
}

impl std::fmt::Display for GameInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (AppID: {}) - {} minutes played",
            self.name,
            self.app_id,
            self.playtime_forever
        )
    }
} 