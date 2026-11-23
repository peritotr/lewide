use serde::Serialize;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors cross the IPC boundary, so they serialize as `{ kind, message }`.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Error {
    #[error("unknown driver: {0}")]
    UnknownDriver(String),
    #[error("invalid connection settings: {0}")]
    InvalidConfig(String),
    #[error("connection failed: {0}")]
    Connect(String),
    /// The SSH server is not in `known_hosts`; the user has to approve its key.
    #[error("unknown SSH host key {fingerprint}")]
    UnknownHostKey {
        fingerprint: String,
        /// Algorithm name as SSH reports it, e.g. `ssh-ed25519`.
        key_type: String,
    },
    /// The SSH server presented a different key than the one in `known_hosts`.
    /// Never recoverable from the application: the user has to check with the
    /// server's administrator.
    #[error("the SSH host key of {host} has changed")]
    HostKeyChanged {
        host: String,
        /// Line of the recorded key in `known_hosts`.
        line: usize,
        saved: String,
        received: String,
    },
    /// The connection to the database is gone; the payload is what the failed call reported.
    #[error("the connection was lost: {0}")]
    ConnectionLost(String),
    #[error("no response after {0} s")]
    Timeout(u64),
    /// A saved connection needs a password that is not stored; the payload is its id.
    #[error("the password for this connection is not stored")]
    CredentialsRequired(String),
    #[error("saved connections: {0}")]
    Storage(String),
    /// A file the user chose could not be written or read.
    #[error("file: {0}")]
    File(String),
    /// An AI provider request failed. `reason` is one of `not_configured`,
    /// `auth`, `rate_limit`, `model`, `request`, `refused`, `server`,
    /// `network`, `cancelled`.
    #[error("{message}")]
    Ai { reason: String, message: String },
    /// The database rejected a statement, with what it said about it.
    #[error("{message}")]
    Database {
        /// The engine's error code: SQLSTATE for PostgreSQL, `number (SQLSTATE)` for MySQL.
        code: Option<String>,
        message: String,
        /// Where in the statement, as a 1-based character offset, when the server says.
        position: Option<u32>,
        /// The 1-based line, for servers that give a line but no offset.
        line: Option<u32>,
        hint: Option<String>,
        detail: Option<String>,
    },
    #[error("query failed: {0}")]
    Query(String),
    #[error("not supported by this driver: {0}")]
    Unsupported(&'static str),
    #[error("no such session: {0}")]
    UnknownSession(u64),
    #[error("{0}")]
    Internal(String),
}
