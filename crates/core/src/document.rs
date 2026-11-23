//! Document databases (MongoDB and similar): collections of JSON-like documents.
//!
//! A driver for such a store implements [`DocumentStore`] beside [`Connection`](crate::Connection)
//! and hands it out through the matching accessor on `Connection`.
//!
//! # How a document travels
//!
//! Documents are returned as JSON values in MongoDB's canonical Extended JSON
//! (version 2). Strings, booleans, null, objects and arrays are plain JSON.
//! Every other type is an object with one `$` key, and numbers are always
//! wrapped, so the stored type of each value can be read back without a second
//! structure beside the document:
//!
//! | stored type  | JSON                                                    |
//! |--------------|---------------------------------------------------------|
//! | `int32`      | `{"$numberInt": "42"}`                                  |
//! | `int64`      | `{"$numberLong": "9007199254740993"}`                   |
//! | `double`     | `{"$numberDouble": "1.5"}` (also `"NaN"`, `"Infinity"`) |
//! | `decimal128` | `{"$numberDecimal": "12.50"}`                           |
//! | `objectId`   | `{"$oid": "65f1c0a2b7e4d3a1c0ffee01"}`                  |
//! | `date`       | `{"$date": {"$numberLong": "1700000000000"}}` (ms, UTC) |
//! | `binData`    | `{"$binary": {"base64": "AP8=", "subType": "00"}}`      |
//! | `regex`      | `{"$regularExpression": {"pattern": "^a", "options": "i"}}` |
//! | `timestamp`  | `{"$timestamp": {"t": 1700000000, "i": 1}}`             |
//!
//! [`type_name`] gives the type of any such value. Fields keep the order the
//! database stores them in.
//!
//! The canonical form was chosen over the relaxed one because relaxed JSON
//! writes `1` for an int32, an int64 and (in JavaScript) a double alike, and
//! an editor that sends such a document back would change the stored types.
//!
//! # Text the user writes
//!
//! Filters, projections, sort orders and documents to store are given as text.
//! Drivers accept Extended JSON, canonical or relaxed, and may accept more (the
//! MongoDB driver also takes unquoted keys, single quotes and `ObjectId("…")`
//! style constructors). Text that cannot be read is reported with
//! [`invalid_text`], which says where the problem is.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatabaseInfo {
    pub name: String,
    /// Space the database takes on disk, when the server reports it.
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionKind {
    Collection,
    /// A stored query; it can be read but not written.
    View,
    Timeseries,
}

/// A collection as the object list and the view bar show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CollectionInfo {
    pub name: String,
    pub kind: CollectionKind,
    /// The numbers below come from the collection's metadata, not from a scan.
    /// They are estimates: after an unclean shutdown, or while documents are
    /// being written, they can be off. `None` where the server keeps none
    /// (views) or the user may not read them.
    pub document_count: Option<u64>,
    /// Average size of one document, in bytes, uncompressed.
    pub avg_document_bytes: Option<u64>,
    /// Size of all documents together, in bytes, uncompressed.
    pub size_bytes: Option<u64>,
}

/// One page of a query. The four text fields hold query documents as the user
/// wrote them; an empty or missing one means "no condition" / "all fields" /
/// "the collection's own order".
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FindRequest {
    pub database: String,
    pub collection: String,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub projection: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    /// Matching documents to pass over before the first one returned.
    #[serde(default)]
    pub skip: u64,
    /// Documents per page. Read as at least 1 and at most [`MAX_PAGE`], so a
    /// mistyped limit cannot load a whole collection into memory.
    pub limit: u32,
}

/// The largest page [`DocumentStore::find`] returns.
pub const MAX_PAGE: u32 = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FoundDocument {
    /// The document in canonical Extended JSON; see the module description.
    pub document: serde_json::Value,
    /// Size of the document as stored (BSON), in bytes. With a projection it
    /// is the size of the projected part.
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FindPage {
    pub documents: Vec<FoundDocument>,
    pub skip: u64,
    /// Matching documents exist beyond this page.
    pub has_more: bool,
    pub elapsed_ms: u64,
}

/// One change to one document. Existing documents are addressed by their
/// `_id`, given as the value [`FoundDocument::document`] holds for it (for
/// example `{"$oid": "…"}`), so the type of the key is kept.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DocumentChange {
    /// Stores a new document. The server gives it an `_id` when it has none.
    Insert {
        document: String,
    },
    /// Replaces every field but `_id`. `document` may leave `_id` out; when it
    /// is there it has to be the same value, since an `_id` cannot change.
    Replace {
        id: serde_json::Value,
        document: String,
    },
    /// Changes part of a document with update operators,
    /// e.g. `{"$set": {"price": 12}, "$unset": {"draft": ""}}`.
    Update {
        id: serde_json::Value,
        update: String,
    },
    Delete {
        id: serde_json::Value,
    },
}

/// What a [`DocumentChange`] did. A replace, update or delete whose `_id` no
/// longer exists is not an error: it reports `matched: 0` or `deleted: 0`, and
/// the caller decides how to tell the user.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WriteOutcome {
    /// Documents the `_id` selected: 0 or 1.
    pub matched: u64,
    /// Documents that actually changed; 0 when the new content equals the old.
    pub modified: u64,
    pub deleted: u64,
    /// The `_id` of an inserted document, in canonical Extended JSON.
    pub inserted_id: Option<serde_json::Value>,
}

#[async_trait]
pub trait DocumentStore: Send + Sync {
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>>;

    /// The collections of a database, sorted by name, with size estimates.
    async fn list_collections(&self, database: &str) -> Result<Vec<CollectionInfo>>;

    /// One collection's entry, for refreshing the view bar without listing the rest.
    async fn collection_info(&self, database: &str, collection: &str) -> Result<CollectionInfo>;

    async fn find(&self, request: FindRequest) -> Result<FindPage>;

    /// Counts the documents a filter matches, exactly. It reads every matching
    /// index entry or document, so it can be slow; pages do not wait for it.
    async fn count(&self, database: &str, collection: &str, filter: Option<&str>) -> Result<u64>;

    async fn write(
        &self,
        database: &str,
        collection: &str,
        change: DocumentChange,
    ) -> Result<WriteOutcome>;
}

/// Which piece of user-written text an [`invalid_text`] error is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPart {
    Filter,
    Projection,
    Sort,
    Document,
    Update,
    Command,
}

impl TextPart {
    /// The `code` of the error for this part: `invalid_filter`, `invalid_projection`,
    /// `invalid_sort`, `invalid_document`, `invalid_update`, `invalid_command`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Filter => "invalid_filter",
            Self::Projection => "invalid_projection",
            Self::Sort => "invalid_sort",
            Self::Document => "invalid_document",
            Self::Update => "invalid_update",
            Self::Command => "invalid_command",
        }
    }
}

/// The error for text that could not be read, before anything was sent to the
/// server.
///
/// It is an [`Error::Database`] because that variant already carries a
/// position: `code` names the field (see [`TextPart::code`]), `message` says
/// what is wrong without a trailing full stop (`a value is expected after
/// $lt:`), and `position` is the 1-based character offset in the text, when
/// known. The interface puts them together as "Filter is not valid: {message}
/// at position {position}".
pub fn invalid_text(part: TextPart, message: impl Into<String>, position: Option<u32>) -> Error {
    Error::Database {
        code: Some(part.code().into()),
        message: message.into(),
        position,
        line: None,
        hint: None,
        detail: None,
    }
}

/// The stored type of a value inside a document, by the names the tree view
/// shows: `string`, `double`, `int32`, `int64`, `decimal128`, `bool`, `date`,
/// `objectId`, `null`, `object`, `array`, `binData`, `regex`, `timestamp`,
/// `javascript`, `javascriptWithScope`, `symbol`, `undefined`, `minKey`,
/// `maxKey`, `dbPointer`.
///
/// Meant for canonical Extended JSON. A bare JSON number (relaxed form) is
/// read the way MongoDB reads one: a whole number is `int32` if it fits and
/// `int64` otherwise, anything else is `double`.
///
/// An object counts as a wrapper only when it has exactly the wrapper's keys,
/// so `{"$oid": "…", "note": 1}` is an `object`. A stored sub-document that
/// happens to look exactly like a wrapper cannot be told apart from one; that
/// is a limit of Extended JSON itself.
pub fn type_name(value: &serde_json::Value) -> &'static str {
    use serde_json::Value;
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => "int32",
            Some(_) => "int64",
            None if n.is_u64() => "int64",
            None => "double",
        },
        Value::Object(map) => {
            if map.len() == 2 && map.contains_key("$code") && map.contains_key("$scope") {
                return "javascriptWithScope";
            }
            if map.len() != 1 {
                return "object";
            }
            match map.keys().next().map(String::as_str) {
                Some("$oid") => "objectId",
                Some("$numberInt") => "int32",
                Some("$numberLong") => "int64",
                Some("$numberDouble") => "double",
                Some("$numberDecimal") => "decimal128",
                Some("$date") => "date",
                Some("$binary" | "$uuid") => "binData",
                Some("$regularExpression") => "regex",
                Some("$timestamp") => "timestamp",
                Some("$code") => "javascript",
                Some("$symbol") => "symbol",
                Some("$undefined") => "undefined",
                Some("$minKey") => "minKey",
                Some("$maxKey") => "maxKey",
                Some("$dbPointer") => "dbPointer",
                _ => "object",
            }
        }
    }
}

/// A document field as one cell of a table export (CSV). Extended JSON
/// wrappers become the plain value they stand for: numbers as numbers, an
/// ObjectId as its hex text, a date as an ISO 8601 UTC time. Objects and lists
/// stay JSON, which CSV writes as text.
pub fn cell(value: &serde_json::Value) -> crate::Value {
    use crate::Value as Cell;
    use serde_json::Value;
    let number = |text: &str| -> Cell {
        if let Ok(int) = text.parse::<i64>() {
            Cell::Int(int)
        } else {
            Cell::Decimal(text.to_owned())
        }
    };
    match value {
        Value::Null => Cell::Null,
        Value::Bool(flag) => Cell::Bool(*flag),
        Value::String(text) => Cell::Text(text.clone()),
        Value::Number(n) => match n.as_i64() {
            Some(int) => Cell::Int(int),
            None => Cell::Decimal(n.to_string()),
        },
        Value::Object(map) if map.len() == 1 => {
            let (key, inner) = map.iter().next().expect("one entry");
            match (key.as_str(), inner) {
                (
                    "$numberInt" | "$numberLong" | "$numberDouble" | "$numberDecimal",
                    Value::String(text),
                ) => number(text),
                ("$oid" | "$symbol", Value::String(text)) => Cell::Text(text.clone()),
                ("$date", Value::String(iso)) => Cell::Temporal(iso.clone()),
                ("$date", Value::Object(ms)) => match ms
                    .get("$numberLong")
                    .and_then(Value::as_str)
                    .and_then(|t| t.parse::<i64>().ok())
                {
                    Some(ms) => Cell::Temporal(iso_time(ms)),
                    None => Cell::Json(value.clone()),
                },
                _ => Cell::Json(value.clone()),
            }
        }
        other => Cell::Json(other.clone()),
    }
}

/// Milliseconds since 1970 as `2023-11-14T22:13:20.000Z`.
fn iso_time(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    // Howard Hinnant's days-to-civil conversion.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn documents_become_cells() {
        use crate::Value as Cell;
        assert_eq!(cell(&json!({"$numberInt": "42"})), Cell::Int(42));
        assert_eq!(
            cell(&json!({"$numberDouble": "1.5"})),
            Cell::Decimal("1.5".into())
        );
        assert_eq!(
            cell(&json!({"$oid": "65f1c0a2b7e4d3a1c0ffee01"})),
            Cell::Text("65f1c0a2b7e4d3a1c0ffee01".into())
        );
        assert_eq!(
            cell(&json!({"$date": {"$numberLong": "1700000000000"}})),
            Cell::Temporal("2023-11-14T22:13:20.000Z".into())
        );
        assert_eq!(
            cell(&json!({"$date": {"$numberLong": "-1"}})),
            Cell::Temporal("1969-12-31T23:59:59.999Z".into())
        );
        assert_eq!(cell(&json!(2.5)), Cell::Decimal("2.5".into()));
        assert_eq!(cell(&json!({"a": 1})), Cell::Json(json!({"a": 1})));
        assert_eq!(cell(&json!(null)), Cell::Null);
    }

    #[test]
    fn names_the_type_of_extended_json_values() {
        let cases = [
            (json!(null), "null"),
            (json!(true), "bool"),
            (json!("text"), "string"),
            (json!([1, 2]), "array"),
            (json!({"a": 1}), "object"),
            (json!({}), "object"),
            (json!({"$numberInt": "1"}), "int32"),
            (json!({"$numberLong": "1"}), "int64"),
            (json!({"$numberDouble": "1.5"}), "double"),
            (json!({"$numberDecimal": "1.50"}), "decimal128"),
            (json!({"$oid": "65f1c0a2b7e4d3a1c0ffee01"}), "objectId"),
            (json!({"$date": {"$numberLong": "0"}}), "date"),
            (
                json!({"$binary": {"base64": "", "subType": "00"}}),
                "binData",
            ),
            (
                json!({"$regularExpression": {"pattern": "a", "options": ""}}),
                "regex",
            ),
            (json!({"$timestamp": {"t": 1, "i": 1}}), "timestamp"),
            (json!({"$code": "f()"}), "javascript"),
            (json!({"$code": "f()", "$scope": {}}), "javascriptWithScope"),
            (json!({"$minKey": 1}), "minKey"),
            // Relaxed numbers.
            (json!(7), "int32"),
            (json!(3_000_000_000_i64), "int64"),
            (json!(1.5), "double"),
            // Not wrappers: an extra key, and an operator that only looks like one.
            (json!({"$oid": "x", "note": 1}), "object"),
            (json!({"$set": {"a": 1}}), "object"),
        ];
        for (value, expected) in cases {
            assert_eq!(type_name(&value), expected, "{value}");
        }
    }

    #[test]
    fn invalid_text_carries_field_and_position() {
        let error = invalid_text(TextPart::Filter, "a value is expected after $lt:", Some(49));
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["kind"], "database");
        assert_eq!(json["message"]["code"], "invalid_filter");
        assert_eq!(json["message"]["position"], 49);
    }

    #[test]
    fn changes_are_read_from_tagged_json() {
        let change: DocumentChange = serde_json::from_value(
            json!({"op": "delete", "id": {"$oid": "65f1c0a2b7e4d3a1c0ffee01"}}),
        )
        .unwrap();
        assert_eq!(
            change,
            DocumentChange::Delete {
                id: json!({"$oid": "65f1c0a2b7e4d3a1c0ffee01"})
            }
        );
    }
}
