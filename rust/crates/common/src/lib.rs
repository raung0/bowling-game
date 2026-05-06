use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    CreateLobby,
    JoinLobby {
        code: String,
        username: String,
    },
    ReconnectPlayer {
        code: String,
        player_session: String,
    },
    ReconnectHost {
        code: String,
        host_session: String,
    },
    Leave,
    StartGame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    LobbyCreated {
        code: String,
        host_session: String,
        players: Vec<PlayerInfo>,
    },
    LobbyJoined {
        code: String,
        player_session: String,
        players: Vec<PlayerInfo>,
    },
    LobbyUpdated {
        code: String,
        players: Vec<PlayerInfo>,
    },
    ReconnectOkHost {
        code: String,
        players: Vec<PlayerInfo>,
    },
    ReconnectOkPlayer {
        code: String,
        player_session: String,
        players: Vec<PlayerInfo>,
    },
    GameStarted {
        code: String,
    },
    Info {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub username: String,
    pub connected: bool,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("server error: {0}")]
    ServerError(String),
    #[error("unknown error")]
    Unknown,
}

impl ClientMessage {
    pub fn encode(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

impl FromStr for ServerMessage {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(serde_json::from_str(s)?)
    }
}

impl ServerMessage {
    pub fn encode(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}
