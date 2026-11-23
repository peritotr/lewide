use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::Value;

/// How a database organizes data. Decides which view the UI opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataModel {
    Relational,
    Document,
    KeyValue,
    Vector,
    Graph,
    TimeSeries,
    Search,
}

/// What a driver can do. The UI enables features from these flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Accepts SQL text.
    pub sql: bool,
    /// Has more than one namespace level (schemas inside a database).
    pub schemas: bool,
    pub transactions: bool,
    /// A running statement can be interrupted.
    pub cancel: bool,
    pub vector_search: bool,
    /// Opens a local file instead of a network address.
    pub file_based: bool,
    /// Supports encrypted connections, see [`TlsConfig`].
    pub tls: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverInfo {
    /// Stable identifier stored in saved connections, e.g. `sqlite`.
    pub id: &'static str,
    pub name: &'static str,
    pub model: DataModel,
    pub capabilities: Capabilities,
    pub default_port: Option<u16>,
    /// Depends on a native client library that is unavailable on mobile.
    pub desktop_only: bool,
}

/// Everything needed to open a connection.
///
/// The secret fields (`password` and those inside [`SshAuth`]) are read but
/// never written: serializing a config always leaves them out, so settings can
/// be stored or sent to the interface without leaking them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub driver: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Database name, or the file path for file-based drivers.
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Reach the server through an SSH server instead of connecting directly.
    #[serde(default)]
    pub ssh: Option<SshConfig>,
    /// Driver-specific options.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// Set by the tunnel layer: connect to this address instead of `host:port`.
    /// `host` stays the real server name so certificates are checked against it.
    #[serde(skip)]
    pub route: Option<SocketAddr>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// No encryption.
    #[default]
    Off,
    /// Encrypt and check the server certificate and host name.
    Verify,
    /// Encrypt without checking who the server is. For self-signed development servers.
    Insecure,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TlsConfig {
    #[serde(default)]
    pub mode: TlsMode,
    /// PEM file with extra certificate authorities to trust, on top of the built-in public ones.
    #[serde(default)]
    pub ca_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub user: String,
    pub auth: SshAuth,
    /// OpenSSH `known_hosts` file used to recognize the server. `None` uses `~/.ssh/known_hosts`.
    #[serde(default)]
    pub known_hosts_file: Option<String>,
    /// Fingerprint (`SHA256:...`) the user agreed to trust for a server that is not in `known_hosts`.
    #[serde(default)]
    pub accept_fingerprint: Option<String>,
    /// Record an approved key in `known_hosts`, as `ssh` does, so it is not asked for again.
    #[serde(default)]
    pub remember_host: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SshAuth {
    Password {
        #[serde(default, skip_serializing)]
        password: String,
    },
    KeyFile {
        path: String,
        #[serde(default, skip_serializing)]
        passphrase: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryRequest {
    pub statement: String,
    /// Stop reading after this many rows; `None` reads everything.
    #[serde(default)]
    pub max_rows: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Column {
    pub name: String,
    /// Type name as the database reports it; `None` for computed columns.
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    /// Set for statements that return no rows.
    pub rows_affected: Option<u64>,
    /// More rows were available than `max_rows` allowed.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Namespace {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    View,
    Collection,
    Index,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DbObject {
    pub name: String,
    pub kind: ObjectKind,
    /// The database's own estimate of the row count, when it keeps one. It
    /// can be far off for tables that were never analyzed.
    pub row_estimate: Option<u64>,
}

/// One page of a table, for browsing it without loading it whole.
#[derive(Debug, Clone, Deserialize)]
pub struct TableRequest {
    #[serde(default)]
    pub namespace: Option<String>,
    pub table: String,
    #[serde(default)]
    pub offset: u64,
    pub limit: u32,
    /// Sort columns, most significant first. The primary key is always
    /// appended, so pages do not overlap.
    #[serde(default)]
    pub order_by: Vec<SortColumn>,
    /// A condition as it would follow `WHERE`, written by the user.
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SortColumn {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TablePage {
    pub columns: Vec<ColumnDescription>,
    pub rows: Vec<Vec<Value>>,
    pub offset: u64,
    /// Rows exist beyond this page.
    pub has_more: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnDescription {
    pub name: String,
    pub type_name: Option<String>,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub primary_key: bool,
    /// The comment stored for the column, where the database keeps one.
    #[serde(default)]
    pub comment: Option<String>,
    /// The allowed values, in order, when the column's type is an enumeration.
    #[serde(default)]
    pub enum_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexDescription {
    pub name: String,
    /// Indexed columns in order; an expression is given as its text.
    pub columns: Vec<String>,
    pub unique: bool,
    /// Backs the primary key.
    pub primary: bool,
    /// The access method (`btree`, `gin`, ...), where the database reports it.
    #[serde(default)]
    pub method: Option<String>,
    /// The index's size as the server words it, e.g. `42 MB`.
    #[serde(default)]
    pub size: Option<String>,
    /// The predicate of a partial index.
    #[serde(default)]
    pub condition: Option<String>,
}

/// A foreign key in another table that points at this one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reference {
    pub name: Option<String>,
    /// `None` when the referencing table is in the same namespace.
    pub namespace: Option<String>,
    pub table: String,
    /// The referencing table's columns.
    pub columns: Vec<String>,
    /// This table's columns they point at.
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

/// A foreign key of a table: its columns, and the columns they point at.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForeignKey {
    /// `None` where the database keeps no name for it (SQLite).
    pub name: Option<String>,
    pub columns: Vec<String>,
    /// `None` when the referenced table is in the same namespace.
    pub referenced_namespace: Option<String>,
    pub referenced_table: String,
    /// Empty when the key points at the primary key without naming it.
    pub referenced_columns: Vec<String>,
    /// The rule as SQL writes it: `CASCADE`, `SET NULL`, `NO ACTION`...
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriggerDescription {
    pub name: String,
    /// `BEFORE`, `AFTER` or `INSTEAD OF`, when the database reports it apart
    /// from the definition.
    pub timing: Option<String>,
    /// `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`.
    pub events: Vec<String>,
    /// The statement that creates it, or its body where that is all there is.
    pub definition: Option<String>,
}

/// Everything the structure view of one table or view shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableStructure {
    pub name: String,
    pub columns: Vec<ColumnDescription>,
    pub indexes: Vec<IndexDescription>,
    pub foreign_keys: Vec<ForeignKey>,
    /// Foreign keys in other tables that reference this one.
    #[serde(default)]
    pub referenced_by: Vec<Reference>,
    pub triggers: Vec<TriggerDescription>,
    /// The statements that would create the object again.
    pub ddl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObjectDescription {
    pub name: String,
    pub columns: Vec<ColumnDescription>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_config_contains_no_secrets() {
        let config = ConnectionConfig {
            driver: "postgres".into(),
            host: Some("db.internal".into()),
            user: Some("app".into()),
            password: Some("db-secret".into()),
            ssh: Some(SshConfig {
                host: "bastion".into(),
                port: None,
                user: "ops".into(),
                auth: SshAuth::KeyFile {
                    path: "/keys/id".into(),
                    passphrase: Some("key-secret".into()),
                },
                known_hosts_file: None,
                accept_fingerprint: None,
                remember_host: false,
            }),
            ..ConnectionConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("db-secret") && !json.contains("key-secret"),
            "{json}"
        );
        assert!(json.contains("db.internal") && json.contains("/keys/id"));

        let with_password = SshAuth::Password {
            password: "ssh-secret".into(),
        };
        let json = serde_json::to_string(&with_password).unwrap();
        assert_eq!(json, r#"{"method":"password"}"#);
        // What was written reads back, with the secrets empty.
        let back: SshAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back,
            SshAuth::Password {
                password: String::new()
            }
        );
    }
}
