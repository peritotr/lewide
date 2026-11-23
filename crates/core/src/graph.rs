//! Graph databases (Neo4j and similar): nodes and relationships queried with Cypher.
//!
//! A driver for such a store implements [`GraphStore`] beside [`Connection`](crate::Connection)
//! and hands it out through `Connection::graph`.
//!
//! A statement's result is given twice: as a table (every row, one column per
//! returned variable) and as a graph (each node and relationship once), so the
//! same answer can fill a grid and a canvas.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Result, Value};

/// Property values as plain JSON. Integers too large for a JavaScript number
/// (beyond ±2^53) and non-finite floats are strings; dates, times, durations
/// and points are strings in the database's own text form.
pub type Properties = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphNode {
    /// The database's identifier for the node (Neo4j: its element id). Stable
    /// while the node exists; use it for expanding neighbours.
    pub id: String,
    pub labels: Vec<String>,
    pub properties: Properties,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphRelationship {
    pub id: String,
    #[serde(rename = "type")]
    pub rel_type: String,
    /// Id of the node the relationship starts at.
    pub start: String,
    /// Id of the node it points to.
    pub end: String,
    pub properties: Properties,
}

/// One cell of the table form of a result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphCell {
    /// A scalar, or a list or map as [`Value::Json`]. Nodes and relationships
    /// inside a list or map appear there as JSON objects of the same shape as
    /// [`GraphNode`] and [`GraphRelationship`].
    Value {
        value: Value,
    },
    /// The whole node, not a reference: the table has every row, also those
    /// whose nodes were left out of the capped graph part.
    Node(GraphNode),
    Relationship(GraphRelationship),
    /// A path in walking order: `nodes[i]` and `nodes[i + 1]` are joined by
    /// `relationships[i]`, which may point either way.
    Path {
        nodes: Vec<GraphNode>,
        relationships: Vec<GraphRelationship>,
    },
}

/// The distinct nodes and relationships of a result, for drawing.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    /// Only relationships whose two ends are both in `nodes`.
    pub relationships: Vec<GraphRelationship>,
}

/// What a statement changed, as the database counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GraphCounters {
    pub nodes_created: u64,
    pub nodes_deleted: u64,
    pub relationships_created: u64,
    pub relationships_deleted: u64,
    pub properties_set: u64,
    pub labels_added: u64,
    pub labels_removed: u64,
    /// Indexes and constraints added or removed, and changes to the system database.
    pub schema_changes: u64,
}

impl GraphCounters {
    /// The sum of all counters; zero for a statement that only read.
    pub fn total(&self) -> u64 {
        self.nodes_created
            + self.nodes_deleted
            + self.relationships_created
            + self.relationships_deleted
            + self.properties_set
            + self.labels_added
            + self.labels_removed
            + self.schema_changes
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CypherRequest {
    /// One statement, or several separated by `;`. Each runs in its own transaction.
    pub statement: String,
    /// Values for `$name` placeholders, shared by all statements in the text.
    #[serde(default)]
    pub parameters: serde_json::Map<String, serde_json::Value>,
    /// The database to run in; `None` uses the one the connection was opened with.
    #[serde(default)]
    pub database: Option<String>,
    /// Keep at most this many rows of each result; `None` keeps all.
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Put at most this many nodes into the graph part; `None` puts all. The
    /// table is not affected.
    #[serde(default)]
    pub node_limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CypherResult {
    /// Names of the returned variables, in order.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<GraphCell>>,
    /// More rows were available than `max_rows` allowed.
    pub truncated: bool,
    /// The nodes and relationships found in `rows`, each once, in order of
    /// first appearance. When a relationship is returned without its end
    /// nodes, those are fetched and added so the relationship can be drawn.
    pub graph: GraphData,
    /// Distinct nodes in the result, including those left out of `graph`.
    pub node_count: u64,
    /// Distinct relationships in the result, including those left out of `graph`.
    pub relationship_count: u64,
    /// Nodes left out of `graph` because of `node_limit`.
    pub nodes_omitted: u64,
    /// Relationships left out of `graph` because one of their ends is not in it.
    pub relationships_omitted: u64,
    /// `None` when the server did not report counters.
    pub counters: Option<GraphCounters>,
    pub elapsed_ms: u64,
}

/// A node label or a relationship type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelInfo {
    pub name: String,
    /// How many nodes carry the label, or relationships have the type. `None`
    /// when the database could not give the number cheaply.
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphDatabase {
    pub name: String,
    /// The server's word for its state, e.g. `online`.
    pub status: Option<String>,
    /// The database used when a session names none.
    pub is_default: bool,
    /// Holds users and settings rather than a graph.
    pub is_system: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Both,
    /// Relationships that start at the node.
    Outgoing,
    /// Relationships that point to the node.
    Incoming,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeighbourRequest {
    #[serde(default)]
    pub database: Option<String>,
    /// Id of the node to expand, as in [`GraphNode::id`].
    pub node_id: String,
    /// Follow at most this many relationships.
    pub limit: u32,
    /// Follow only relationships of this type.
    #[serde(default)]
    pub rel_type: Option<String>,
    #[serde(default)]
    pub direction: Direction,
}

/// How many relationships of one type a node has in one direction.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationshipGroup {
    #[serde(rename = "type")]
    pub rel_type: String,
    /// `true` when they start at the node. A relationship from the node to
    /// itself is counted once, as outgoing.
    pub outgoing: bool,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Neighbours {
    /// The nodes at the other end of `relationships`, each once. The expanded
    /// node itself is included only when it has a relationship to itself.
    pub nodes: Vec<GraphNode>,
    /// At most `limit` relationships of the node.
    pub relationships: Vec<GraphRelationship>,
    /// How many relationships match the request in all, for "3 shown of 14".
    pub total: u64,
    /// Every relationship of the node by type and direction, whatever the
    /// request filtered on.
    pub groups: Vec<RelationshipGroup>,
}

#[async_trait]
pub trait GraphStore: Send + Sync {
    /// Runs every statement in the text and returns one result per statement, in order.
    async fn run(&self, request: CypherRequest) -> Result<Vec<CypherResult>>;

    async fn databases(&self) -> Result<Vec<GraphDatabase>>;

    /// Node labels of `database` (the connection's own if `None`), sorted by name.
    async fn labels(&self, database: Option<&str>) -> Result<Vec<LabelInfo>>;

    /// Relationship types of `database` (the connection's own if `None`), sorted by name.
    async fn relationship_types(&self, database: Option<&str>) -> Result<Vec<LabelInfo>>;

    /// The relationships of one node and the nodes at their other ends.
    async fn neighbours(&self, request: NeighbourRequest) -> Result<Neighbours>;
}
