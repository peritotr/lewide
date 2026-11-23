//! Turns what the document view's fields hold into SQL++ (N1QL) pieces, and
//! cuts query-tab text into statements. Pure functions.
//!
//! The view's fields take what people bring from MongoDB and from Couchbase:
//! a filter is a `{"field": value}` shorthand (with MongoDB's `$gt`, `$in`…)
//! or SQL++ text that follows `WHERE`; a sort is `{"field": 1 | -1}` or text
//! that follows `ORDER BY`. Values reach the server as JSON literals, which
//! SQL++ reads as they are.

use dbclient_core::Error;
use dbclient_core::document::{TextPart, invalid_text};
use serde_json::{Map, Value as Json};

/// The alias of the collection in every statement the driver builds.
pub const ALIAS: &str = "d";

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

/// A name in backticks; a backtick inside is doubled.
pub fn ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// `d.\`a\`.\`b\`[0]` for the path `a.b.0`: number parts index into lists.
pub fn path(field: &str) -> Result<String, Error> {
    let mut out = ALIAS.to_owned();
    for (i, part) in field.split('.').enumerate() {
        if part.is_empty() {
            return Err(Error::Query(format!("{field}: a field name is empty")));
        }
        if i > 0 && part.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str(&format!("[{part}]"));
        } else {
            out.push('.');
            out.push_str(&ident(part));
        }
    }
    Ok(out)
}

/// A JSON value as a SQL++ literal.
fn literal(value: &Json) -> String {
    value.to_string()
}

/// The condition that follows `WHERE`, or `None` for every document.
pub fn filter(text: Option<&str>) -> Result<Option<String>, Error> {
    let Some(text) = blank(text) else {
        return Ok(None);
    };
    if !text.starts_with('{') {
        // SQL++ as written. A semicolon would let it carry another statement.
        if text.contains(';') {
            return Err(invalid_text(
                TextPart::Filter,
                "a filter cannot contain ';'",
                None,
            ));
        }
        return Ok(Some(format!("({text}\n)")));
    }
    let Json::Object(fields) = plain(parse_json(TextPart::Filter, text)?) else {
        return Err(invalid_text(
            TextPart::Filter,
            "a filter is an object",
            Some(1),
        ));
    };
    let mut parts = Vec::new();
    for (field, value) in &fields {
        let at = path(field).map_err(|e| invalid_text(TextPart::Filter, e.to_string(), None))?;
        match value {
            // Like MongoDB: null also matches a document without the field.
            Json::Null => parts.push(format!("({at} IS NULL OR {at} IS MISSING)")),
            Json::Object(ops) if !ops.is_empty() && ops.keys().all(|k| k.starts_with('$')) => {
                for (op, operand) in ops {
                    parts.push(operator(&at, op, operand)?);
                }
            }
            // A list value matches when the field holds it or is one of it, as MongoDB's equality does.
            value => parts.push(format!(
                "({at} = {v} OR (IS_ARRAY({at}) AND {v} WITHIN {at}))",
                v = literal(value)
            )),
        }
    }
    Ok((!parts.is_empty()).then(|| parts.join(" AND ")))
}

fn operator(at: &str, op: &str, operand: &Json) -> Result<String, Error> {
    let compare = |symbol: &str| format!("{at} {symbol} {}", literal(operand));
    Ok(match op {
        "$eq" => compare("="),
        "$ne" => format!("({at} IS MISSING OR {at} != {})", literal(operand)),
        "$gt" => compare(">"),
        "$gte" => compare(">="),
        "$lt" => compare("<"),
        "$lte" => compare("<="),
        "$in" | "$nin" => {
            if !operand.is_array() {
                return Err(invalid_text(
                    TextPart::Filter,
                    format!("{op} needs a list"),
                    None,
                ));
            }
            let test = format!("{at} IN {}", literal(operand));
            if op == "$in" {
                test
            } else {
                format!("NOT ({test})")
            }
        }
        "$exists" => {
            if operand.as_bool() == Some(false) {
                format!("{at} IS MISSING")
            } else {
                format!("{at} IS NOT MISSING")
            }
        }
        other => {
            return Err(invalid_text(
                TextPart::Filter,
                format!("{other} is not supported here; write the condition in SQL++ instead"),
                None,
            ));
        }
    })
}

/// The text that follows `ORDER BY`, or `None` for the key order.
pub fn sort(text: Option<&str>) -> Result<Option<String>, Error> {
    let Some(text) = blank(text) else {
        return Ok(None);
    };
    if !text.starts_with('{') {
        if text.contains(';') {
            return Err(invalid_text(
                TextPart::Sort,
                "a sort cannot contain ';'",
                None,
            ));
        }
        return Ok(Some(text.to_owned()));
    }
    let Json::Object(fields) = parse_json(TextPart::Sort, text)? else {
        return Err(invalid_text(TextPart::Sort, "a sort is an object", Some(1)));
    };
    let parts = fields
        .iter()
        .map(|(field, order)| {
            let descending = match order {
                Json::Number(n) => n.as_f64().is_some_and(|v| v < 0.0),
                Json::String(s) => s.eq_ignore_ascii_case("desc"),
                _ => false,
            };
            let at = if field == "_id" {
                format!("META({ALIAS}).id")
            } else {
                path(field).map_err(|e| invalid_text(TextPart::Sort, e.to_string(), None))?
            };
            Ok(format!("{at} {}", if descending { "DESC" } else { "ASC" }))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok((!parts.is_empty()).then(|| parts.join(", ")))
}

/// Which top-level fields a projection keeps: `{field: 1}` keeps the named
/// ones, `{field: 0}` drops them, a list keeps those in it.
pub enum Projection {
    All,
    Keep(Vec<String>),
    Drop(Vec<String>),
}

pub fn projection(text: Option<&str>) -> Result<Projection, Error> {
    let Some(text) = blank(text) else {
        return Ok(Projection::All);
    };
    match parse_json(TextPart::Projection, text)? {
        Json::Array(fields) => Ok(Projection::Keep(
            fields
                .iter()
                .filter_map(|f| f.as_str().map(str::to_owned))
                .collect(),
        )),
        Json::Object(fields) => {
            let (mut keep, mut drop) = (Vec::new(), Vec::new());
            for (field, value) in fields {
                let off = matches!(&value, Json::Number(n) if n.as_f64() == Some(0.0))
                    || value == Json::Bool(false);
                if field == "_id" {
                    continue;
                }
                if off {
                    drop.push(field)
                } else {
                    keep.push(field)
                }
            }
            if !keep.is_empty() && !drop.is_empty() {
                return Err(invalid_text(
                    TextPart::Projection,
                    "a projection either keeps fields or drops them, not both",
                    Some(1),
                ));
            }
            Ok(if keep.is_empty() {
                Projection::Drop(drop)
            } else {
                Projection::Keep(keep)
            })
        }
        _ => Err(invalid_text(
            TextPart::Projection,
            "a projection is a list of fields or an object",
            Some(1),
        )),
    }
}

impl Projection {
    /// Applies the projection to a stored document's fields. Paths with dots
    /// are followed into objects.
    pub fn apply(&self, fields: Map<String, Json>) -> Map<String, Json> {
        match self {
            Projection::All => fields,
            Projection::Keep(keep) => {
                let source = Json::Object(fields);
                let mut out = Map::new();
                for name in keep {
                    let mut at = &source;
                    let mut found = true;
                    for part in name.split('.') {
                        match at.get(part) {
                            Some(next) => at = next,
                            None => {
                                found = false;
                                break;
                            }
                        }
                    }
                    if found {
                        out.insert(name.clone(), at.clone());
                    }
                }
                out
            }
            Projection::Drop(drop) => fields
                .into_iter()
                .filter(|(name, _)| !drop.contains(name))
                .collect(),
        }
    }
}

/// Turns MongoDB Extended JSON, which the document view's editors write, into
/// the plain JSON Couchbase stores: `{"$numberDouble": "1.5"}` is `1.5`,
/// `{"$date": …}` an ISO date string, `{"$oid": …}` its text.
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
                        // Canonical form: milliseconds since 1970.
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

/// Applies `$set` paths to a stored document, as MongoDB does: missing
/// objects on the way are created, number parts index into lists.
pub fn apply_set(document: &mut Json, set: Vec<(String, Json)>) -> Result<(), Error> {
    for (path, value) in set {
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

/// The `$set` part of the view's `{"$set": {"a.b.0": value}}`.
pub fn set_paths(update: &str) -> Result<Vec<(String, Json)>, Error> {
    let parsed = parse_json(TextPart::Update, update)?;
    let Some(Json::Object(set)) = parsed.get("$set") else {
        return Err(invalid_text(
            TextPart::Update,
            "only {\"$set\": {…}} updates are supported",
            Some(1),
        ));
    };
    if set.contains_key("_id") {
        return Err(invalid_text(
            TextPart::Update,
            "the _id is the document's key and cannot change",
            None,
        ));
    }
    Ok(set
        .iter()
        .map(|(k, v)| (k.clone(), plain(v.clone())))
        .collect())
}

/// One statement and the byte offset where it starts in the whole text.
#[derive(Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub text: &'a str,
    pub offset: usize,
}

/// Cuts text on semicolons outside strings (`'…'`, `"…"`, with backslash
/// escapes), backtick names and comments.
pub fn statements<'a>(text: &'a str) -> Vec<Piece<'a>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let push = |out: &mut Vec<Piece<'a>>, from: usize, to: usize| {
        let piece = &text[from..to];
        let code = strip_comments(piece);
        if !code.trim().is_empty() {
            let lead = piece.len() - piece.trim_start().len();
            out.push(Piece {
                text: &text[from + lead..from + piece.trim_end().len()],
                offset: from + lead,
            });
        }
    };
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && quote != b'`' {
                        i += 2;
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = text[i..].find('\n').map_or(bytes.len(), |p| i + p);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            b';' => {
                push(&mut out, start, i);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    push(&mut out, start, bytes.len());
    out
}

/// The text without `--` and `/* */` comments, for telling whether code is left.
fn strip_comments(piece: &str) -> String {
    let mut out = String::new();
    let mut rest = piece;
    loop {
        let line = rest.find("--");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |p| &rest[l + p..]);
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                rest = rest[b + 2..]
                    .find("*/")
                    .map_or("", |p| &rest[b + 2 + p + 2..]);
            }
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_and_names() {
        assert_eq!(path("city").unwrap(), "d.`city`");
        assert_eq!(path("items.0.name").unwrap(), "d.`items`[0].`name`");
        assert_eq!(path("we`ird").unwrap(), "d.`we``ird`");
        assert!(path("a..b").is_err());
    }

    #[test]
    fn filters() {
        assert_eq!(filter(None).unwrap(), None);
        assert_eq!(filter(Some("{}")).unwrap(), None);
        assert_eq!(
            filter(Some("city = \"İzmir\"")).unwrap().unwrap(),
            "(city = \"İzmir\"\n)"
        );
        assert!(filter(Some("1=1; DELETE FROM x")).is_err());
        assert_eq!(
            filter(Some(
                r#"{"city": "İzmir", "total": {"$gte": 5, "$lt": 20}}"#
            ))
            .unwrap()
            .unwrap(),
            "(d.`city` = \"İzmir\" OR (IS_ARRAY(d.`city`) AND \"İzmir\" WITHIN d.`city`)) AND d.`total` >= 5 AND d.`total` < 20"
        );
        assert_eq!(
            filter(Some(r#"{"note": null}"#)).unwrap().unwrap(),
            "(d.`note` IS NULL OR d.`note` IS MISSING)"
        );
        assert_eq!(
            filter(Some(r#"{"n": {"$numberInt": "3"}}"#))
                .unwrap()
                .unwrap(),
            "(d.`n` = 3 OR (IS_ARRAY(d.`n`) AND 3 WITHIN d.`n`))"
        );
        assert!(filter(Some(r#"{"a": {"$regex": "x"}}"#)).is_err());
        let Err(Error::Database { code, position, .. }) = filter(Some("{\"a\": }")) else {
            panic!("bad JSON must say where");
        };
        assert_eq!(code.as_deref(), Some("invalid_filter"));
        assert!(position.is_some());
    }

    #[test]
    fn sorts_and_projections() {
        assert_eq!(
            sort(Some(r#"{"total": -1, "_id": 1}"#)).unwrap().unwrap(),
            "d.`total` DESC, META(d).id ASC"
        );
        assert_eq!(sort(Some("total DESC")).unwrap().unwrap(), "total DESC");
        let doc = json!({"a": 1, "b": {"c": 2}, "e": 3});
        let Json::Object(fields) = doc else {
            unreachable!()
        };
        let kept = projection(Some(r#"{"a": 1, "b.c": 1}"#))
            .unwrap()
            .apply(fields.clone());
        assert_eq!(Json::Object(kept), json!({"a": 1, "b.c": 2}));
        let dropped = projection(Some(r#"{"e": 0}"#)).unwrap().apply(fields);
        assert_eq!(Json::Object(dropped), json!({"a": 1, "b": {"c": 2}}));
        assert!(projection(Some(r#"{"a": 1, "e": 0}"#)).is_err());
    }

    #[test]
    fn set_paths_turn_extended_json_plain() {
        let set =
            set_paths(r#"{"$set": {"total": {"$numberDouble": "1.5"}, "items.0": "x"}}"#).unwrap();
        assert_eq!(
            set,
            [("total".into(), json!(1.5)), ("items.0".into(), json!("x"))]
        );
        assert!(set_paths(r#"{"$unset": {"a": ""}}"#).is_err());
        assert!(set_paths(r#"{"$set": {"_id": "x"}}"#).is_err());
        let mut doc = json!({"items": [1, 2]});
        apply_set(
            &mut doc,
            vec![
                ("items.1".into(), json!(9)),
                ("address.city".into(), json!("İzmir")),
            ],
        )
        .unwrap();
        assert_eq!(doc, json!({"items": [1, 9], "address": {"city": "İzmir"}}));
        assert!(apply_set(&mut doc, vec![("items.5".into(), json!(1))]).is_err());
    }

    #[test]
    fn splits_statements() {
        let text = "SELECT 'a;b' AS x;\n-- c;\nSELECT \"\\\";\" FROM `we;ird`; /* ; */ ";
        let pieces: Vec<&str> = statements(text).iter().map(|p| p.text).collect();
        assert_eq!(
            pieces,
            ["SELECT 'a;b' AS x", "-- c;\nSELECT \"\\\";\" FROM `we;ird`"]
        );
        assert_eq!(statements(text)[1].offset, text.find("-- c").unwrap());
    }
}
