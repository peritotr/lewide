//! Turns the Query API's typed JSON into the client's values.
//!
//! With `Accept: application/vnd.neo4j.query` every value arrives as
//! `{"$type": "...", "_value": ...}`. That is what tells a node from a map
//! that merely has the same keys, and it carries integers as text so none
//! lose precision on the way.

use std::collections::HashSet;

use base64::Engine;
use dbclient_core::Value;
use dbclient_core::graph::{GraphCell, GraphNode, GraphRelationship, Properties};
use serde_json::Value as Json;

/// The largest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn parts(typed: &Json) -> Option<(&str, &Json)> {
    Some((typed.get("$type")?.as_str()?, typed.get("_value")?))
}

fn text(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// A top-level result value as a table cell.
pub fn cell(typed: &Json) -> GraphCell {
    let Some((kind, value)) = parts(typed) else {
        return GraphCell::Value {
            value: Value::Json(typed.clone()),
        };
    };
    let scalar = match kind {
        "Node" => match node(typed) {
            Some(node) => return GraphCell::Node(node),
            None => Value::Json(value.clone()),
        },
        "Relationship" => match relationship(typed) {
            Some(relationship) => return GraphCell::Relationship(relationship),
            None => Value::Json(value.clone()),
        },
        "Path" => {
            let (nodes, relationships) = path(value);
            return GraphCell::Path {
                nodes,
                relationships,
            };
        }
        "Null" => Value::Null,
        "Boolean" => value.as_bool().map_or(Value::Null, Value::Bool),
        "Integer" => match text(value).parse() {
            Ok(int) => Value::Int(int),
            Err(_) => Value::Text(text(value)),
        },
        // NaN and the infinities have no JSON number; they stay as the server's words.
        "Float" => match text(value).parse::<f64>() {
            Ok(float) if float.is_finite() => Value::Float(float),
            _ => Value::Text(text(value)),
        },
        "Base64" => match base64::engine::general_purpose::STANDARD.decode(text(value)) {
            Ok(bytes) => Value::Bytes(bytes),
            Err(_) => Value::Text(text(value)),
        },
        "Date" | "Time" | "LocalTime" | "OffsetDateTime" | "ZonedDateTime" | "LocalDateTime" => {
            Value::Temporal(text(value))
        }
        "List" | "Map" => Value::Json(plain(typed)),
        // String, Duration, Point, and types added after this was written.
        _ => match value {
            Json::String(text) => Value::Text(text.clone()),
            other => Value::Json(other.clone()),
        },
    };
    GraphCell::Value { value: scalar }
}

/// A value as plain JSON, for properties and for what is inside lists and maps.
pub fn plain(typed: &Json) -> Json {
    let Some((kind, value)) = parts(typed) else {
        return typed.clone();
    };
    match kind {
        "Null" => Json::Null,
        "Integer" => match text(value).parse::<i64>() {
            Ok(int) if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&int) => Json::from(int),
            _ => Json::String(text(value)),
        },
        "Float" => text(value)
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| Json::String(text(value)), Json::Number),
        "List" => Json::Array(
            value
                .as_array()
                .map(|items| items.iter().map(plain).collect())
                .unwrap_or_default(),
        ),
        "Map" => Json::Object(properties(value)),
        "Node" => node(typed).map_or(Json::Null, |n| serde_json::json!(n)),
        "Relationship" => relationship(typed).map_or(Json::Null, |r| serde_json::json!(r)),
        "Path" => {
            let (nodes, relationships) = path(value);
            serde_json::json!({ "nodes": nodes, "relationships": relationships })
        }
        _ => value.clone(),
    }
}

fn properties(map: &Json) -> Properties {
    map.as_object()
        .map(|map| map.iter().map(|(k, v)| (k.clone(), plain(v))).collect())
        .unwrap_or_default()
}

pub fn node(typed: &Json) -> Option<GraphNode> {
    let (kind, value) = parts(typed)?;
    if kind != "Node" {
        return None;
    }
    Some(GraphNode {
        id: value.get("_element_id")?.as_str()?.to_owned(),
        labels: value
            .get("_labels")
            .and_then(Json::as_array)
            .map(|labels| labels.iter().map(text).collect())
            .unwrap_or_default(),
        properties: properties(value.get("_properties").unwrap_or(&Json::Null)),
    })
}

pub fn relationship(typed: &Json) -> Option<GraphRelationship> {
    let (kind, value) = parts(typed)?;
    if kind != "Relationship" {
        return None;
    }
    let field = |name: &str| Some(value.get(name)?.as_str()?.to_owned());
    Some(GraphRelationship {
        id: field("_element_id")?,
        rel_type: field("_type")?,
        start: field("_start_node_element_id")?,
        end: field("_end_node_element_id")?,
        properties: properties(value.get("_properties").unwrap_or(&Json::Null)),
    })
}

/// A path arrives as one list: node, relationship, node, and so on.
fn path(value: &Json) -> (Vec<GraphNode>, Vec<GraphRelationship>) {
    let items = value.as_array().map(Vec::as_slice).unwrap_or_default();
    (
        items.iter().filter_map(node).collect(),
        items.iter().filter_map(relationship).collect(),
    )
}

/// A cell for the plain grid, which knows nothing of nodes: they become JSON.
pub fn grid_value(typed: &Json) -> Value {
    match cell(typed) {
        GraphCell::Value { value } => value,
        _ => Value::Json(plain(typed)),
    }
}

/// An unsigned number from a typed integer, for counts.
pub fn count(typed: &Json) -> Option<u64> {
    let (_, value) = parts(typed)?;
    text(value).parse().ok()
}

/// A string from a typed value; `None` for null.
pub fn string(typed: &Json) -> Option<String> {
    match parts(typed)? {
        ("Null", _) => None,
        (_, value) => Some(text(value)),
    }
}

pub fn boolean(typed: &Json) -> Option<bool> {
    parts(typed)?.1.as_bool()
}

/// Gathers the distinct nodes and relationships of a result as it is read.
pub struct Collector {
    /// Nodes beyond this many are counted but not kept.
    limit: Option<usize>,
    node_ids: HashSet<String>,
    pub nodes: Vec<GraphNode>,
    relationship_ids: HashSet<String>,
    pub relationships: Vec<GraphRelationship>,
}

impl Collector {
    pub fn new(limit: Option<usize>) -> Self {
        Collector {
            limit,
            node_ids: HashSet::new(),
            nodes: Vec::new(),
            relationship_ids: HashSet::new(),
            relationships: Vec::new(),
        }
    }

    /// Distinct nodes seen, kept or not.
    pub fn nodes_seen(&self) -> usize {
        self.node_ids.len()
    }

    pub fn has_node(&self, id: &str) -> bool {
        self.node_ids.contains(id)
    }

    /// How many more nodes fit under the limit.
    pub fn room(&self) -> usize {
        self.limit
            .map_or(usize::MAX, |limit| limit.saturating_sub(self.nodes.len()))
    }

    /// Looks through a value, also inside lists, maps and paths.
    pub fn visit(&mut self, typed: &Json) {
        let Some((kind, value)) = parts(typed) else {
            return;
        };
        match kind {
            "Node" => {
                let Some(id) = value.get("_element_id").and_then(Json::as_str) else {
                    return;
                };
                if self.node_ids.contains(id) {
                    return;
                }
                self.node_ids.insert(id.to_owned());
                if self.room() > 0 {
                    self.nodes.extend(node(typed));
                }
            }
            "Relationship" => {
                let Some(id) = value.get("_element_id").and_then(Json::as_str) else {
                    return;
                };
                if self.relationship_ids.insert(id.to_owned()) {
                    self.relationships.extend(relationship(typed));
                }
            }
            "List" | "Path" => {
                for item in value.as_array().into_iter().flatten() {
                    self.visit(item);
                }
            }
            "Map" => {
                for item in value.as_object().into_iter().flat_map(|map| map.values()) {
                    self.visit(item);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn typed_node(id: &str) -> Json {
        json!({"$type": "Node", "_value": {
            "_element_id": id,
            "_labels": ["Person"],
            "_properties": {
                "name": {"$type": "String", "_value": "Ada"},
                "big": {"$type": "Integer", "_value": "9007199254740993"},
                "born": {"$type": "Date", "_value": "1815-12-10"},
            },
        }})
    }

    fn typed_relationship(id: &str, start: &str, end: &str) -> Json {
        json!({"$type": "Relationship", "_value": {
            "_element_id": id,
            "_start_node_element_id": start,
            "_end_node_element_id": end,
            "_type": "KNOWS",
            "_properties": {"since": {"$type": "Integer", "_value": "1840"}},
        }})
    }

    #[test]
    fn scalars_become_core_values() {
        let value = |kind: &str, value: Json| match cell(&json!({"$type": kind, "_value": value})) {
            GraphCell::Value { value } => value,
            other => panic!("{other:?}"),
        };
        assert_eq!(value("Null", Json::Null), Value::Null);
        assert_eq!(value("Boolean", json!(true)), Value::Bool(true));
        assert_eq!(
            value("Integer", json!("9223372036854775807")),
            Value::Int(i64::MAX)
        );
        assert_eq!(value("Float", json!("1.5")), Value::Float(1.5));
        assert_eq!(value("Float", json!("NaN")), Value::Text("NaN".into()));
        assert_eq!(value("String", json!("x")), Value::Text("x".into()));
        assert_eq!(value("Base64", json!("AP8=")), Value::Bytes(vec![0, 255]));
        assert_eq!(
            value("LocalDateTime", json!("2026-01-02T03:04:05")),
            Value::Temporal("2026-01-02T03:04:05".into())
        );
        assert_eq!(value("Duration", json!("P1D")), Value::Text("P1D".into()));
        assert_eq!(
            value(
                "List",
                json!([{"$type": "Integer", "_value": "1"}, {"$type": "Null", "_value": null}])
            ),
            Value::Json(json!([1, null]))
        );
    }

    #[test]
    fn nodes_and_relationships_keep_their_identity() {
        let GraphCell::Node(node) = cell(&typed_node("4:x:0")) else {
            panic!("not a node");
        };
        assert_eq!(node.id, "4:x:0");
        assert_eq!(node.labels, ["Person"]);
        // Too large for a JavaScript number: sent as text.
        assert_eq!(node.properties["big"], json!("9007199254740993"));
        assert_eq!(node.properties["born"], json!("1815-12-10"));

        let GraphCell::Relationship(relationship) =
            cell(&typed_relationship("5:x:0", "4:x:0", "4:x:1"))
        else {
            panic!("not a relationship");
        };
        assert_eq!(relationship.rel_type, "KNOWS");
        assert_eq!(relationship.start, "4:x:0");
        assert_eq!(relationship.end, "4:x:1");
        assert_eq!(relationship.properties["since"], json!(1840));
    }

    #[test]
    fn cells_serialize_with_a_kind() {
        let json =
            serde_json::to_value(cell(&typed_relationship("5:x:0", "4:x:0", "4:x:1"))).unwrap();
        assert_eq!(
            json,
            json!({"kind": "relationship", "id": "5:x:0", "type": "KNOWS",
                   "start": "4:x:0", "end": "4:x:1", "properties": {"since": 1840}})
        );
        let json = serde_json::to_value(cell(&json!({"$type": "Integer", "_value": "7"}))).unwrap();
        assert_eq!(
            json,
            json!({"kind": "value", "value": {"type": "int", "value": "7"}})
        );
    }

    #[test]
    fn collector_keeps_each_item_once_and_counts_past_the_limit() {
        let path = json!({"$type": "Path", "_value": [
            typed_node("4:x:0"),
            typed_relationship("5:x:0", "4:x:0", "4:x:1"),
            typed_node("4:x:1"),
        ]});
        let nested = json!({"$type": "Map", "_value": {
            "list": {"$type": "List", "_value": [typed_node("4:x:1"), typed_node("4:x:2")]},
        }});

        let mut all = Collector::new(None);
        all.visit(&path);
        all.visit(&nested);
        all.visit(&typed_node("4:x:0"));
        assert_eq!(all.nodes.len(), 3);
        assert_eq!(all.relationships.len(), 1);

        let mut capped = Collector::new(Some(2));
        capped.visit(&path);
        capped.visit(&nested);
        assert_eq!(capped.nodes.len(), 2);
        assert_eq!(capped.nodes_seen(), 3);
        assert_eq!(capped.room(), 0);
    }
}
