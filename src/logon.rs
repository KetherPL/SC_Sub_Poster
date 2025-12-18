// SPDX-License-Identifier: LGPL-3.0-only

use std::error::Error;
use std::net::IpAddr;
use crate::errors::{classify_connection_error, ErrorDomain, ErrorInventoryEntry, RetryDisposition};
use thiserror::Error;
use tracing::{info, instrument};
use steam_vent::auth::{
    AuthConfirmationHandler, ConsoleAuthConfirmationHandler, DeviceConfirmationHandler,
    FileGuardDataStore,
};
use steam_vent::{Connection, ConnectionTrait, ServerList};
use steamid_ng2::SteamID;

/// Steam client wrapper for authenticated and anonymous operations
pub struct KetherSteamClient {
    connection: Connection,
}

impl KetherSteamClient {
    /// Create a new Steam client with provided credentials
    #[instrument(name = "kether.logon.new", skip(password))]
    pub async fn new(account: &str, password: &str) -> Result<Self, Box<dyn Error>> {
        let server_list = bootstrap::discover_servers()
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(LogonError::from(err)) })?;
        let connection = bootstrap::credential_login(&server_list, account, password)
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(LogonError::from(err)) })?;

        let connection = Self::validate_and_finalize_connection(connection)?;

        info!(steam_id = %connection.steam_id().steam3(), "logon successful");

        Ok(Self { connection })
    }

    /// Create an anonymous Steam client for testing
    #[instrument(name = "kether.logon.new_anonymous")]
    pub async fn new_anonymous() -> Result<Self, Box<dyn Error>> {
        let server_list = bootstrap::discover_servers()
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(LogonError::from(err)) })?;
        let connection = bootstrap::anonymous_login(&server_list)
            .await
            .map_err(|err| -> Box<dyn Error> { Box::new(LogonError::from(err)) })?;

        let connection = Self::validate_and_finalize_connection(connection)?;

        info!(steam_id = %connection.steam_id().steam3(), "anonymous logon successful");

        Ok(Self { connection })
    }

    /// Common validation and finalization logic for connections
    fn validate_and_finalize_connection(connection: Connection) -> Result<Connection, Box<dyn Error>> {
        ensure_valid_connection(&connection)
            .map_err(|err| -> Box<dyn Error> { Box::new(err) })?;
        Ok(connection)
    }

    /// Get the Steam ID of the connected user
    pub fn steam_id(&self) -> SteamID {
        self.connection.steam_id()
    }

    /// Get the connection for direct access to Steam services
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Obtain a read-only snapshot of the session without exposing the connection.
    pub fn session_snapshot(&self) -> SessionSnapshot {
        SessionSnapshot::from_connection(&self.connection)
    }

    /// Get a mutable reference to the connection
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Test if the connection is working by requesting app info
    #[instrument(name = "kether.logon.test_connection", skip(self))]
    pub async fn test_connection(&self) -> Result<(), Box<dyn Error>> {
        use steam_vent::proto::steammessages_clientserver_appinfo::{
            cmsg_client_picsproduct_info_request, CMsgClientPICSProductInfoRequest,
            CMsgClientPICSProductInfoResponse,
        };
        
        // Request basic app info for TF2 (appid 440) - lightweight test that works for both authenticated and anonymous
        let req = CMsgClientPICSProductInfoRequest {
            apps: vec![cmsg_client_picsproduct_info_request::AppInfo {
                appid: Some(440),
                only_public_obsolete: Some(true),
                ..Default::default()
            }],
            meta_data_only: Some(true), // Only request metadata, not full app data
            single_response: Some(true),
            ..Default::default()
        };
        
        let _response: CMsgClientPICSProductInfoResponse = self.connection.job(req).await?;
        info!("connection round-trip succeeded");
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

mod bootstrap {
    use super::*;

    pub async fn discover_servers() -> Result<ServerList, steam_vent::ServerDiscoveryError> {
        ServerList::discover().await
    }

    pub async fn credential_login(
        server_list: &ServerList,
        account: &str,
        password: &str,
    ) -> Result<Connection, steam_vent::ConnectionError> {
        Connection::login(
            server_list,
            account,
            password,
            FileGuardDataStore::user_cache(),
            ConsoleAuthConfirmationHandler::default().or(DeviceConfirmationHandler),
        )
        .await
    }

    pub async fn anonymous_login(
        server_list: &ServerList,
    ) -> Result<Connection, steam_vent::ConnectionError> {
        Connection::anonymous(server_list).await
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        #[ignore = "Requires Steam network access"]
        async fn discover_and_login_anonymous() {
            let servers = discover_servers().await.expect("discover servers");
            let connection = anonymous_login(&servers)
                .await
                .expect("anonymous login");
            assert_ne!(connection.steam_id().account_id(), 0);

            let snapshot = SessionSnapshot::from_connection(&connection);
            assert_eq!(snapshot.steam_id, connection.steam_id());
        }
    }
}

/// Information about a Steam game
#[derive(Debug, Clone)]
pub struct GameInfo {
    pub app_id: u32,
    pub name: String,
    pub playtime_forever: u32,
}

/// Immutable snapshot of connection/session state.
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub steam_id: SteamID,
    pub session_id: i32,
    pub cell_id: u32,
    pub public_ip: Option<IpAddr>,
    pub ip_country_code: Option<String>,
    pub access_token: Option<String>,
}

impl SessionSnapshot {
    pub fn from_connection(connection: &Connection) -> Self {
        Self {
            steam_id: connection.steam_id(),
            session_id: connection.session_id(),
            cell_id: connection.cell_id(),
            public_ip: connection.public_ip(),
            ip_country_code: connection.ip_country_code(),
            access_token: connection.access_token().map(|token| token.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum LogonError {
    #[error("failed to discover Steam servers: {source}")]
    Discovery {
        #[source]
        source: steam_vent::ServerDiscoveryError,
        inventory: ErrorInventoryEntry,
    },
    #[error("failed to establish connection: {source}")]
    Connection {
        #[source]
        source: steam_vent::ConnectionError,
        inventory: ErrorInventoryEntry,
    },
    #[error("invalid session state: {message}")]
    InvariantViolation {
        message: &'static str,
        inventory: ErrorInventoryEntry,
    },
}

impl LogonError {
    pub fn inventory(&self) -> ErrorInventoryEntry {
        match self {
            LogonError::Discovery { inventory, .. }
            | LogonError::Connection { inventory, .. }
            | LogonError::InvariantViolation { inventory, .. } => *inventory,
        }
    }

    fn discovery_err(source: steam_vent::ServerDiscoveryError) -> Self {
        LogonError::Discovery {
            source,
            inventory: ErrorInventoryEntry::new(
                ErrorDomain::Transport,
                RetryDisposition::BackoffRetry,
                "server discovery failed",
            ),
        }
    }

    fn invariant(message: &'static str) -> Self {
        LogonError::InvariantViolation {
            message,
            inventory: ErrorInventoryEntry::new(
                ErrorDomain::Application,
                RetryDisposition::Fatal,
                message,
            ),
        }
    }
}

impl From<steam_vent::ServerDiscoveryError> for LogonError {
    fn from(value: steam_vent::ServerDiscoveryError) -> Self {
        LogonError::discovery_err(value)
    }
}

impl From<steam_vent::ConnectionError> for LogonError {
    fn from(value: steam_vent::ConnectionError) -> Self {
        let inventory = classify_connection_error(&value);
        LogonError::Connection {
            source: value,
            inventory,
        }
    }
}

fn ensure_valid_connection(connection: &Connection) -> Result<(), LogonError> {
    let steam_id = connection.steam_id();
    if steam_id.account_id() == 0 {
        return Err(LogonError::invariant("steam ID missing after login"));
    }

    if connection.session_id() == 0 {
        return Err(LogonError::invariant("session ID not assigned"));
    }

    Ok(())
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