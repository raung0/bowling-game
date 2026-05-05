use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Pong,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawResponse {
    status: String,
    response: Option<Response>,
    value: Option<String>,
}

impl Response {
    pub fn from_str(s: &str) -> Result<Response, Error> {
        let raw: RawResponse = serde_json::from_str(s)?;
        if raw.status != "ok" {
            return Err(Error::ServerError(
                raw.value.unwrap_or("no error value provided".into()),
            ));
        } else if let Some(v) = raw.response {
            Ok(v)
        } else {
            Err(Error::Unknown)
        }
    }
}

impl Request {
    pub fn encode(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}
