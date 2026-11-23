//! Driver-agnostic core of the database client.
//!
//! Every database is reached through the [`Driver`] and [`Connection`] traits.
//! A driver declares what it can do through [`Capabilities`]; the UI reads
//! those instead of branching on driver names.

pub mod document;
mod driver;
pub mod edit;
mod error;
pub mod export;
pub mod graph;
pub mod import;
pub mod kv;
mod model;
mod registry;
pub mod sql;
pub mod structure;
mod value;
pub mod vector;
pub mod xlsx;

pub use driver::{Connection, Driver};
pub use edit::{ApplyResult, ChangeSet, RowChange};
pub use error::{Error, Result};
pub use model::{
    Capabilities, Column, ColumnDescription, ConnectionConfig, DataModel, DbObject, DriverInfo,
    ForeignKey, IndexDescription, Namespace, ObjectDescription, ObjectKind, QueryRequest,
    QueryResult, Reference, SortColumn, SshAuth, SshConfig, TablePage, TableRequest,
    TableStructure, TlsConfig, TlsMode, TriggerDescription,
};
pub use registry::DriverRegistry;
pub use sql::SqlDialect;
pub use value::Value;
