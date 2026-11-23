//! Key-value stores: listing keys and reading and changing one key's value.
//!
//! A driver for such a store implements [`KeyValue`] beside [`Connection`](crate::Connection)
//! and hands it out through `Connection::key_value`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Result, Value};

/// One key as the key list shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyEntry {
    pub key: String,
    /// The store's own name for the type: `string`, `hash`, `list`, `set`, `zset`, `stream`...
    pub kind: String,
    /// Milliseconds until the key expires; `None` when it does not.
    pub ttl_ms: Option<i64>,
}

/// A step of an incremental listing. Keys are found by scanning, never by a
/// command that walks the whole keyspace at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanPage {
    pub keys: Vec<KeyEntry>,
    /// Where to continue; `"0"` when the scan is complete.
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyspaceInfo {
    /// How many keys the database says it holds.
    pub keys: u64,
    pub memory_bytes: Option<u64>,
    /// Numbers of the databases that hold keys, with their key counts.
    pub databases: Vec<(u32, u64)>,
}

/// A key's value, or the part of it that was asked for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyData {
    String {
        value: Value,
    },
    Hash {
        fields: Vec<(String, Value)>,
        total: u64,
    },
    List {
        items: Vec<Value>,
        /// Index of the first item returned.
        start: u64,
        total: u64,
    },
    Set {
        members: Vec<Value>,
        /// Sets are scanned; `"0"` when every member has been returned.
        cursor: String,
        total: u64,
    },
    Zset {
        members: Vec<(Value, f64)>,
        /// Rank of the first member returned, in the order asked for.
        start: u64,
        total: u64,
    },
    /// A type the client has no view for: only its name is known.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyDetails {
    pub key: String,
    pub kind: String,
    pub ttl_ms: Option<i64>,
    pub size_bytes: Option<u64>,
    pub encoding: Option<String>,
    pub data: KeyData,
}

/// Which part of a key to read. Strings and hashes are read whole.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReadRange {
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub limit: u32,
    /// Sorted sets: highest score first.
    #[serde(default)]
    pub descending: bool,
    /// Sets: where to continue scanning.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// One change to one key. Each is a single command, applied at once.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum KeyCommand {
    /// Replaces a string's value and keeps its time to live.
    SetString {
        key: String,
        value: String,
    },
    HashSet {
        key: String,
        field: String,
        value: String,
    },
    HashDelete {
        key: String,
        field: String,
    },
    ListPush {
        key: String,
        value: String,
        left: bool,
    },
    ListSet {
        key: String,
        index: i64,
        value: String,
    },
    ListRemove {
        key: String,
        index: i64,
    },
    SetAdd {
        key: String,
        member: String,
    },
    SetRemove {
        key: String,
        member: String,
    },
    ZsetAdd {
        key: String,
        member: String,
        score: f64,
    },
    ZsetRemove {
        key: String,
        member: String,
    },
    Delete {
        key: String,
    },
    Rename {
        key: String,
        to: String,
    },
    Expire {
        key: String,
        seconds: u64,
    },
    Persist {
        key: String,
    },
}

#[async_trait]
pub trait KeyValue: Send + Sync {
    async fn keyspace(&self, db: u32) -> Result<KeyspaceInfo>;

    /// One step of a scan. `kind` limits it to one type where the store can.
    async fn scan(
        &self,
        db: u32,
        pattern: &str,
        kind: Option<&str>,
        cursor: &str,
        count: u32,
    ) -> Result<ScanPage>;

    async fn read(&self, db: u32, key: &str, range: ReadRange) -> Result<KeyDetails>;

    /// Runs the change and returns the command as it was sent, for the user to see.
    async fn write(&self, db: u32, command: KeyCommand) -> Result<String>;
}
