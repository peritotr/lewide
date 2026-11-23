//! Vector databases (Qdrant and similar): collections of points searched by similarity.
//!
//! A driver for such a store implements [`VectorStore`] beside [`Connection`](crate::Connection)
//! and hands it out through `Connection::vector`.
//!
//! Everything here reads. The vector view never writes, so the trait has no
//! way to change a collection or a point.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Result;

/// The identifier of a point: an unsigned integer or a UUID.
///
/// Serialized as a string in both cases (`"42"`, `"6f1e7f0a-…"`), because
/// JavaScript numbers lose precision above 2^53 and integer ids use the full
/// 64 bits. Read back from a JSON number or a string; a string that is a
/// whole number in the 64-bit range is an integer id, anything else a UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PointId {
    Number(u64),
    Uuid(String),
}

impl PointId {
    /// Reads an id as a person would type it: digits are an integer id.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        // The largest 64-bit number has 20 digits. Longer runs of digits are a
        // UUID written without hyphens, even when zeros make the number small.
        let digits = text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit());
        match text.parse::<u64>() {
            Ok(number) if digits => PointId::Number(number),
            _ => PointId::Uuid(text.to_owned()),
        }
    }
}

impl fmt::Display for PointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PointId::Number(number) => write!(f, "{number}"),
            PointId::Uuid(uuid) => f.write_str(uuid),
        }
    }
}

impl Serialize for PointId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PointId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(u64),
            Text(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Number(number) => PointId::Number(number),
            Raw::Text(text) => PointId::parse(&text),
        })
    }
}

/// How the values of one vector are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorKind {
    /// One list of numbers of a fixed length.
    Dense,
    /// Several dense vectors of the same length stored as one (late-interaction models).
    Multi,
    /// Only the non-zero positions are stored; there is no fixed length.
    Sparse,
}

/// One vector a collection stores per point.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VectorSpec {
    /// `None` for the single unnamed vector of a collection without named vectors.
    pub name: Option<String>,
    pub kind: VectorKind,
    /// Number of dimensions; `None` for sparse vectors.
    pub size: Option<u64>,
    /// The store's own name for the metric: `Cosine`, `Euclid`, `Dot`, `Manhattan`.
    /// `None` where the store does not report one (sparse vectors).
    pub distance: Option<String>,
    /// Whether a higher score means a closer match. False for distance metrics
    /// (`Euclid`, `Manhattan`), where the score is a distance and the best
    /// match has the lowest one. A minimum score is then an upper limit.
    pub higher_is_better: bool,
}

/// A payload field the store keeps an index for. Fields without an index can
/// still be filtered on; this list only shows which ones the store knows by name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PayloadField {
    pub name: String,
    /// The store's own name for the type: `keyword`, `integer`, `float`, `bool`, `datetime`...
    pub kind: String,
    /// How many points have a value in this field, when the store says.
    pub points: Option<u64>,
}

/// What the view bar shows about a collection.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CollectionInfo {
    pub name: String,
    /// The store's health word for the collection (`green`, `yellow`, `grey`, `red`).
    pub status: Option<String>,
    /// The store's own count. It can lag behind while the collection is being changed.
    pub points: Option<u64>,
    /// One entry for a plain collection, several for one with named vectors.
    /// Unnamed first, then dense and multi by name, then sparse by name.
    pub vectors: Vec<VectorSpec>,
    pub payload_fields: Vec<PayloadField>,
}

/// The values of one vector of a point.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VectorData {
    Dense {
        values: Vec<f32>,
    },
    Multi {
        vectors: Vec<Vec<f32>>,
    },
    Sparse {
        indices: Vec<u32>,
        values: Vec<f32>,
    },
    /// A layout this client does not know, as the store sent it.
    Other {
        raw: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointVector {
    /// `None` for the unnamed vector.
    pub name: Option<String>,
    #[serde(flatten)]
    pub data: VectorData,
}

/// A stored point.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    pub id: PointId,
    /// The payload as the store keeps it, a JSON object. `None` when it was not asked for.
    pub payload: Option<serde_json::Value>,
    /// Empty when vectors were not asked for. The store may return a vector
    /// changed from what was written: Qdrant normalizes vectors of a `Cosine` collection.
    pub vectors: Vec<PointVector>,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoredPoint {
    /// Position in the result, starting at 1 for the best match.
    pub rank: u32,
    pub score: f64,
    #[serde(flatten)]
    pub point: Point,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    /// Best match first.
    pub hits: Vec<ScoredPoint>,
    /// How many of the top K were left out because of the minimum score.
    pub below_min_score: u32,
    pub elapsed_ms: u64,
}

/// A step of a plain listing of points, in the store's own id order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointPage {
    pub points: Vec<Point>,
    /// Where to continue; `None` when every point has been returned.
    pub next: Option<PointId>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    /// The field equals one of the values of a list.
    In,
    /// The field is missing, null or an empty list. Payloads have no schema,
    /// so a point without the field counts as null.
    IsNull,
    IsNotNull,
}

/// One payload condition. A filter is a list of these, all of which must hold.
///
/// Semantics follow the store, not SQL: when the field holds a list, a
/// condition holds if any item satisfies it; and `Ne` also matches points
/// that do not have the field at all.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Condition {
    /// The payload key; nested fields with dots (`address.city`).
    pub field: String,
    pub op: ConditionOp,
    /// What to compare with: a string, number or boolean for `Eq` and `Ne`; a
    /// number or an RFC 3339 date-time string for the ordering operators; a
    /// list for `In`. Ignored by `IsNull` and `IsNotNull`.
    #[serde(default)]
    pub value: serde_json::Value,
}

/// What to search with.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum SearchQuery {
    /// A vector the user typed or an embedding model produced.
    Vector { values: Vec<f32> },
    /// The vector of a stored point ("Find similar"). The point itself is not among the hits.
    Point { id: PointId },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchRequest {
    pub collection: String,
    pub query: SearchQuery,
    /// Which named vector to search; `None` for the unnamed one.
    #[serde(default)]
    pub using: Option<String>,
    /// Top K: how many of the best matches to look at. At least 1.
    pub limit: u32,
    /// Leave out hits scoring worse than this: below it, or above it when the
    /// metric is a distance (see [`VectorSpec::higher_is_better`]).
    #[serde(default)]
    pub min_score: Option<f64>,
    /// Conditions that must all hold. Empty searches every point.
    #[serde(default)]
    pub filter: Vec<Condition>,
    #[serde(default)]
    pub with_payload: bool,
    #[serde(default)]
    pub with_vector: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScrollRequest {
    pub collection: String,
    /// How many points to return. At least 1.
    pub limit: u32,
    /// `next` of the previous page; `None` starts from the beginning.
    #[serde(default)]
    pub cursor: Option<PointId>,
    #[serde(default)]
    pub filter: Vec<Condition>,
    #[serde(default)]
    pub with_payload: bool,
    #[serde(default)]
    pub with_vector: bool,
}

#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Collection names, sorted.
    async fn list_collections(&self) -> Result<Vec<String>>;

    async fn collection_info(&self, collection: &str) -> Result<CollectionInfo>;

    /// The closest points to a vector or to a stored point, best first.
    async fn search(&self, request: SearchRequest) -> Result<SearchResult>;

    /// One point with its payload and every vector it has.
    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Point>;

    /// One page of points without any ranking, for browsing a collection.
    async fn scroll(&self, request: ScrollRequest) -> Result<PointPage>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_ids_travel_as_strings() {
        let big = PointId::Number(u64::MAX);
        assert_eq!(
            serde_json::to_string(&big).unwrap(),
            r#""18446744073709551615""#
        );
        let uuid = PointId::Uuid("6f1e7f0a-0000-4000-8000-000000000001".into());
        assert_eq!(
            serde_json::to_string(&uuid).unwrap(),
            r#""6f1e7f0a-0000-4000-8000-000000000001""#
        );
        for id in [big, uuid] {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<PointId>(&json).unwrap(), id);
        }
    }

    #[test]
    fn point_ids_are_read_from_numbers_and_text() {
        let read = |json: &str| serde_json::from_str::<PointId>(json).unwrap();
        assert_eq!(read("42"), PointId::Number(42));
        assert_eq!(read(r#"" 42 ""#), PointId::Number(42));
        // 32 digits are a UUID without hyphens: too long for an integer id.
        assert_eq!(
            read(r#""00000000000000000000000000000001""#),
            PointId::Uuid("00000000000000000000000000000001".into())
        );
        assert!(serde_json::from_str::<PointId>("-1").is_err());
        assert!(serde_json::from_str::<PointId>("1.5").is_err());
    }

    #[test]
    fn requests_read_from_what_the_interface_sends() {
        let request: SearchRequest = serde_json::from_value(serde_json::json!({
            "collection": "articles",
            "query": { "by": "point", "id": "7" },
            "limit": 10,
            "filter": [
                { "field": "lang", "op": "in", "value": ["en", "de"] },
                { "field": "deleted_at", "op": "is_null" }
            ]
        }))
        .unwrap();
        assert_eq!(
            request.query,
            SearchQuery::Point {
                id: PointId::Number(7)
            }
        );
        assert_eq!(request.filter[1].op, ConditionOp::IsNull);
        assert_eq!(request.filter[1].value, serde_json::Value::Null);
        assert!(!request.with_payload && request.min_score.is_none());

        let request: SearchRequest = serde_json::from_value(serde_json::json!({
            "collection": "articles",
            "query": { "by": "vector", "values": [0.5, -1] },
            "limit": 3
        }))
        .unwrap();
        assert_eq!(
            request.query,
            SearchQuery::Vector {
                values: vec![0.5, -1.0]
            }
        );
    }

    #[test]
    fn a_hit_is_one_flat_object() {
        let hit = ScoredPoint {
            rank: 1,
            score: 0.5,
            point: Point {
                id: PointId::Number(7),
                payload: Some(serde_json::json!({ "title": "a" })),
                vectors: vec![PointVector {
                    name: None,
                    data: VectorData::Dense {
                        values: vec![1.0, 0.25],
                    },
                }],
            },
        };
        assert_eq!(
            serde_json::to_value(&hit).unwrap(),
            serde_json::json!({
                "rank": 1,
                "score": 0.5,
                "id": "7",
                "payload": { "title": "a" },
                "vectors": [{ "name": null, "kind": "dense", "values": [1.0, 0.25] }]
            })
        );
    }
}
