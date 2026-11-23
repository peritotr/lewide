//! Turns what the document view's fields hold into Elasticsearch request
//! bodies, and reads the query tab's console text. Pure functions.

use dbclient_core::Error;
use dbclient_core::document::{TextPart, invalid_text};
use serde_json::{Map, Value as Json, json};

/// Query types a filter may start with. An object whose keys are all field
/// names instead is a shorthand: `{"city": "İzmir"}` matches on each field.
const QUERY_TYPES: &[&str] = &[
    "bool",
    "boosting",
    "combined_fields",
    "constant_score",
    "dis_max",
    "distance_feature",
    "exists",
    "function_score",
    "fuzzy",
    "geo_bounding_box",
    "geo_distance",
    "geo_polygon",
    "geo_shape",
    "has_child",
    "has_parent",
    "ids",
    "intervals",
    "knn",
    "match",
    "match_all",
    "match_bool_prefix",
    "match_none",
    "match_phrase",
    "match_phrase_prefix",
    "more_like_this",
    "multi_match",
    "nested",
    "neural",
    "parent_id",
    "percolate",
    "pinned",
    "prefix",
    "query_string",
    "range",
    "rank_feature",
    "regexp",
    "script",
    "script_score",
    "semantic",
    "simple_query_string",
    "sparse_vector",
    "span_containing",
    "span_first",
    "span_multi",
    "span_near",
    "span_not",
    "span_or",
    "span_term",
    "span_within",
    "term",
    "terms",
    "terms_set",
    "wildcard",
    "wrapper",
];

/// Reads JSON a person typed. The error points at the place, counted in
/// characters from 1, as the field's error line expects.
pub fn parse_json(part: TextPart, text: &str) -> Result<Json, Error> {
    serde_json::from_str(text).map_err(|e| {
        let position = text
            .lines()
            .take(e.line().saturating_sub(1))
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            + e.column();
        invalid_text(part, e.to_string(), Some(position as u32))
    })
}

fn blank(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty() && *t != "{}")
}

/// The `query` of a search: Query DSL, the field shorthand, or Lucene query
/// string syntax for text that is not JSON (`city:İzmir AND year:>2023`).
pub fn query(filter: Option<&str>) -> Result<Json, Error> {
    let Some(text) = blank(filter) else {
        return Ok(json!({ "match_all": {} }));
    };
    if !text.starts_with('{') {
        return Ok(json!({ "query_string": { "query": text } }));
    }
    let Json::Object(object) = parse_json(TextPart::Filter, text)? else {
        return Err(invalid_text(
            TextPart::Filter,
            "a filter is an object",
            Some(1),
        ));
    };
    if object.keys().any(|k| QUERY_TYPES.contains(&k.as_str())) {
        return Ok(Json::Object(object));
    }
    // The shorthand: every field must match. Lists match any of their values.
    let must: Vec<Json> = object
        .into_iter()
        .map(|(field, value)| match value {
            Json::Array(values) => json!({ "terms": { field: values } }),
            Json::Null => json!({ "bool": { "must_not": { "exists": { "field": field } } } }),
            value => json!({ "match": { field: { "query": value, "operator": "and" } } }),
        })
        .collect();
    Ok(json!({ "bool": { "filter": must } }))
}

/// `_source` filtering. A list of fields, or MongoDB's `{field: 1}` /
/// `{field: 0}` form people coming from the document view are used to.
pub fn source(projection: Option<&str>) -> Result<Option<Json>, Error> {
    let Some(text) = blank(projection) else {
        return Ok(None);
    };
    Ok(Some(match parse_json(TextPart::Projection, text)? {
        Json::Array(fields) => json!({ "includes": fields }),
        Json::Object(fields) => {
            let (mut includes, mut excludes) = (Vec::new(), Vec::new());
            for (field, keep) in fields {
                match keep {
                    Json::Number(n) if n.as_f64() == Some(0.0) => excludes.push(field),
                    Json::Bool(false) => excludes.push(field),
                    _ => includes.push(field),
                }
            }
            json!({ "includes": includes, "excludes": excludes })
        }
        _ => {
            return Err(invalid_text(
                TextPart::Projection,
                "a projection is a list of fields or an object",
                Some(1),
            ));
        }
    }))
}

/// `sort`: Elasticsearch's own list, or `{field: 1 | -1 | "asc" | "desc"}`.
pub fn sort(text: Option<&str>) -> Result<Option<Json>, Error> {
    let Some(text) = blank(text) else {
        return Ok(None);
    };
    Ok(Some(match parse_json(TextPart::Sort, text)? {
        Json::Array(list) => Json::Array(list),
        Json::Object(fields) => Json::Array(
            fields
                .into_iter()
                .map(|(field, order)| {
                    let order = match &order {
                        Json::Number(n) if n.as_f64().is_some_and(|v| v < 0.0) => json!("desc"),
                        Json::String(s) if s.eq_ignore_ascii_case("desc") => json!("desc"),
                        Json::Object(_) => order,
                        _ => json!("asc"),
                    };
                    json!({ field: order })
                })
                .collect(),
        ),
        _ => {
            return Err(invalid_text(
                TextPart::Sort,
                "a sort is a list or an object",
                Some(1),
            ));
        }
    }))
}

/// Turns MongoDB Extended JSON, which the document view's editors write,
/// into the plain JSON Elasticsearch stores: `{"$numberDouble": "1.5"}` is
/// `1.5`, `{"$date": …}` an ISO date string, `{"$oid": …}` its text.
pub fn plain(value: Json) -> Json {
    match value {
        Json::Array(items) => Json::Array(items.into_iter().map(plain).collect()),
        Json::Object(map) => {
            if map.len() == 1 {
                let (key, inner) = map.iter().next().expect("one entry");
                let number = |text: &Json| {
                    let text = text.as_str()?;
                    text.parse::<i64>().map(Json::from).ok().or_else(|| {
                        text.parse::<f64>()
                            .ok()
                            .and_then(|f| serde_json::Number::from_f64(f).map(Json::Number))
                    })
                };
                let converted = match key.as_str() {
                    "$numberInt" | "$numberLong" | "$numberDouble" | "$numberDecimal" => {
                        number(inner)
                    }
                    "$oid" | "$symbol" => inner.as_str().map(|s| Json::String(s.to_owned())),
                    "$date" => match inner {
                        Json::String(iso) => Some(Json::String(iso.clone())),
                        // Canonical form: milliseconds since 1970, which Elasticsearch's date fields also take.
                        Json::Object(ms) => ms.get("$numberLong").and_then(number),
                        Json::Number(n) => Some(Json::Number(n.clone())),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(converted) = converted {
                    return converted;
                }
            }
            Json::Object(map.into_iter().map(|(k, v)| (k, plain(v))).collect())
        }
        other => other,
    }
}

/// Applies the document view's `{"$set": {"a.b.0": value}}` to a stored
/// document. Numeric parts index into lists, as in MongoDB paths.
pub fn apply_set(document: &mut Json, update: &str) -> Result<(), Error> {
    let parsed = parse_json(TextPart::Update, update)?;
    let Some(Json::Object(set)) = parsed.get("$set") else {
        return Err(invalid_text(
            TextPart::Update,
            "only {\"$set\": {…}} updates are supported",
            Some(1),
        ));
    };
    for (path, value) in set {
        let value = plain(value.clone());
        let parts: Vec<&str> = path.split('.').collect();
        let (last, parents) = parts.split_last().expect("split gives at least one part");
        let mut at = &mut *document;
        for part in parents {
            at = match at {
                Json::Array(items) => part
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| items.get_mut(i))
                    .ok_or_else(|| Error::Query(format!("{path}: no such item")))?,
                Json::Object(fields) => fields
                    .entry(part.to_string())
                    .or_insert_with(|| Json::Object(Map::new())),
                _ => return Err(Error::Query(format!("{path}: {part} is not an object"))),
            };
        }
        match at {
            Json::Array(items) => {
                let slot = last
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| items.get_mut(i))
                    .ok_or_else(|| Error::Query(format!("{path}: no such item")))?;
                *slot = value;
            }
            Json::Object(fields) => {
                fields.insert(last.to_string(), value);
            }
            _ => return Err(Error::Query(format!("{path}: the parent is not an object"))),
        }
    }
    Ok(())
}

/// One request of the query tab, written as in Kibana's Dev Tools console.
#[derive(Debug, PartialEq)]
pub struct ConsoleRequest {
    pub method: String,
    pub path: String,
    /// The body as written: one JSON value, or NDJSON lines for `_bulk`.
    pub body: Option<String>,
}

const METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "HEAD"];

/// Splits console text into requests. A request starts with a line such as
/// `GET /index/_search`; the lines up to the next one are its body. Lines
/// starting with `#` or `//` outside a body are comments.
pub fn console(text: &str) -> Result<Vec<ConsoleRequest>, Error> {
    let mut requests: Vec<ConsoleRequest> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let finish = |requests: &mut Vec<ConsoleRequest>, body: &mut Vec<&str>| {
        if let Some(last) = requests.last_mut() {
            let text = body.join("\n");
            last.body = Some(text.trim().to_owned()).filter(|b| !b.is_empty());
        }
        body.clear();
    };
    for line in text.lines() {
        let trimmed = line.trim();
        let mut words = trimmed.splitn(2, char::is_whitespace);
        let first = words.next().unwrap_or_default().to_ascii_uppercase();
        if METHODS.contains(&first.as_str()) {
            finish(&mut requests, &mut body);
            let path = words.next().unwrap_or("/").trim();
            let path = if path.starts_with('/') {
                path.to_owned()
            } else {
                format!("/{path}")
            };
            requests.push(ConsoleRequest {
                method: first,
                path,
                body: None,
            });
        } else if requests.is_empty() {
            if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
                return Err(invalid_text(
                    TextPart::Command,
                    "start with a request line such as GET /my-index/_search",
                    Some(1),
                ));
            }
        } else {
            body.push(line);
        }
    }
    finish(&mut requests, &mut body);
    if requests.is_empty() {
        return Err(Error::Query("no request to run".into()));
    }
    Ok(requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_take_dsl_shorthand_and_query_strings() {
        assert_eq!(query(None).unwrap(), json!({"match_all": {}}));
        assert_eq!(query(Some(" {} ")).unwrap(), json!({"match_all": {}}));
        let dsl = r#"{"range": {"year": {"gte": 2024}}}"#;
        assert_eq!(
            query(Some(dsl)).unwrap(),
            serde_json::from_str::<Json>(dsl).unwrap()
        );
        assert_eq!(
            query(Some(
                r#"{"city": "İzmir", "tags": ["a", "b"], "gone": null}"#
            ))
            .unwrap(),
            json!({"bool": {"filter": [
                {"match": {"city": {"query": "İzmir", "operator": "and"}}},
                {"terms": {"tags": ["a", "b"]}},
                {"bool": {"must_not": {"exists": {"field": "gone"}}}}
            ]}})
        );
        assert_eq!(
            query(Some("city:İzmir AND year:>2023")).unwrap(),
            json!({"query_string": {"query": "city:İzmir AND year:>2023"}})
        );
    }

    #[test]
    fn bad_json_points_at_the_place() {
        let Error::Database { code, position, .. } = query(Some("{\n  \"a\": }")).unwrap_err()
        else {
            panic!("not a text error");
        };
        assert_eq!(code.as_deref(), Some("invalid_filter"));
        // Line 2, column 8: after "{\n" (2 characters) comes the 8th.
        assert_eq!(position, Some(10));
    }

    #[test]
    fn projections_and_sorts_read_both_forms() {
        assert_eq!(
            source(Some(r#"["a", "b"]"#)).unwrap(),
            Some(json!({"includes": ["a", "b"]}))
        );
        assert_eq!(
            source(Some(r#"{"a": 1, "b": 0}"#)).unwrap(),
            Some(json!({"includes": ["a"], "excludes": ["b"]}))
        );
        assert_eq!(
            sort(Some(r#"{"year": -1, "title.keyword": 1}"#)).unwrap(),
            Some(json!([{"year": "desc"}, {"title.keyword": "asc"}]))
        );
        assert_eq!(
            sort(Some(r#"[{"year": "desc"}]"#)).unwrap(),
            Some(json!([{"year": "desc"}]))
        );
        assert!(sort(Some("3")).is_err());
    }

    #[test]
    fn set_paths_reach_into_objects_and_lists() {
        let mut doc = json!({"a": {"b": 1}, "tags": ["x", "y"]});
        apply_set(
            &mut doc,
            r#"{"$set": {"a.b": 2, "a.c.d": true, "tags.1": "z"}}"#,
        )
        .unwrap();
        assert_eq!(
            doc,
            json!({"a": {"b": 2, "c": {"d": true}}, "tags": ["x", "z"]})
        );
        assert!(apply_set(&mut doc, r#"{"$set": {"tags.5": 1}}"#).is_err());
        assert!(apply_set(&mut doc, r#"{"$inc": {"a.b": 1}}"#).is_err());
    }

    #[test]
    fn extended_json_from_the_editors_becomes_plain() {
        let mut doc = json!({"price": 1, "when": "x"});
        apply_set(
            &mut doc,
            r#"{"$set": {"price": {"$numberDouble": "12.5"}, "stock": {"$numberInt": "3"}, "when": {"$date": {"$numberLong": "1700000000000"}}}}"#,
        )
        .unwrap();
        assert_eq!(
            doc,
            json!({"price": 12.5, "stock": 3, "when": 1700000000000_i64})
        );
        assert_eq!(
            plain(json!({"a": [{"$oid": "abc"}], "b": {"c": 1}})),
            json!({"a": ["abc"], "b": {"c": 1}})
        );
        // Ordinary one-key objects are left alone.
        assert_eq!(plain(json!({"$custom": 1})), json!({"$custom": 1}));
    }

    #[test]
    fn console_text_is_split_into_requests() {
        let text = "# all documents\nGET my-index/_search\n{\n  \"query\": {\"match_all\": {}}\n}\n\nPOST /_bulk\n{\"index\": {\"_index\": \"i\"}}\n{\"a\": 1}\nget /";
        let requests = console(text).unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/my-index/_search");
        assert!(requests[0].body.as_deref().unwrap().contains("match_all"));
        assert_eq!(
            requests[1].body.as_deref(),
            Some("{\"index\": {\"_index\": \"i\"}}\n{\"a\": 1}")
        );
        assert_eq!(requests[2].method, "GET");
        assert_eq!(requests[2].body, None);
        assert!(console("{\"a\": 1}").is_err());
        assert!(console("  \n# nothing").is_err());
    }
}
