//! Void.rs's independent, version-pinned Minecraft Java 26.2 protocol core.
//! This is an explicit subset, not a declaration of complete vanilla compatibility.
pub mod block_states;
pub mod chunk;
pub mod codec;
mod connection;
mod crypto;
pub mod nbt;
mod status;

pub use chunk::{ChunkData, ChunkSection};
pub use connection::{
    ClientCommand, ConnectOptions, Connection, PlayerPosition, ServerEvent, State, connect_offline,
    connect_online,
};
pub use crypto::OnlineIdentity;
pub use status::{ServerAddress, ServerStatus, status};

pub const MINECRAFT_VERSION: &str = "26.2";
pub const PROTOCOL_VERSION: i32 = 776;
pub const DATA_VERSION: i32 = 4903;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed Minecraft packet: {0}")]
    Malformed(&'static str),
    #[error("Minecraft packet exceeds {0} limit")]
    Limit(&'static str),
    #[error("invalid address: {0}")]
    Address(String),
    #[error("server disconnected: {0}")]
    Disconnected(String),
    #[error("unsupported protocol requirement: {0}")]
    Unsupported(String),
    #[error("server status JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("connection timed out")]
    Timeout,
    #[error("client is not consuming network events quickly enough")]
    Backpressure,
    #[error("Minecraft authentication: {0}")]
    Authentication(String),
}
