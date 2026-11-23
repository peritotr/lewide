//! Values, type names, errors and keys: the parts that need no server.

use dbclient_core::{Error, Value};
use serde_json::Value as Json;

/// The type without the wrappers that do not change how a value reads:
/// `Nullable(LowCardinality(String))` is a `String`.
pub fn base_type(ty: &str) -> &str {
    let mut ty = ty.trim();
    loop {
        let inner = ["Nullable(", "LowCardinality("]
            .iter()
            .find_map(|w| ty.strip_prefix(w).and_then(|rest| rest.strip_suffix(')')));
        match inner {
            Some(inner) => ty = inner.trim(),
            None => return ty,
        }
    }
}

/// One cell of a JSON result as a core value. 64-bit integers and decimals
/// arrive as strings (the driver asks for that), so no digit is lost.
pub fn to_value(value: &Json, ty: &str) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    let ty = base_type(ty);
    let text = || match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    };
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| ty.starts_with(p));
    if starts(&["Int", "UInt"]) {
        return text()
            .parse::<i64>()
            .map_or_else(|_| Value::Decimal(text()), Value::Int);
    }
    if starts(&["Float", "BFloat"]) {
        return match value {
            Json::Number(n) => n.as_f64().map_or(Value::Text(text()), Value::Float),
            _ => text().parse().map_or(Value::Text(text()), Value::Float),
        };
    }
    if starts(&["Decimal"]) {
        return Value::Decimal(text());
    }
    if ty == "Bool" {
        return value.as_bool().map_or(Value::Text(text()), Value::Bool);
    }
    if starts(&["Date", "DateTime"]) {
        return Value::Temporal(text());
    }
    if ty == "Nothing" {
        return Value::Null;
    }
    match value {
        Json::String(s) => Value::Text(s.clone()),
        // Arrays, maps, tuples, JSON and the like.
        other => Value::Json(other.clone()),
    }
}

/// A server error, from the exception code header and the reply body:
/// `Code: 62. DB::Exception: Syntax error: failed at position 8 (FROM): … (SYNTAX_ERROR) (version …)`.
/// `offset` is the statement's start in the editor text, in bytes, and
/// `whole` that text, so the position can be made absolute in characters.
pub fn server_error(code_header: Option<&str>, body: &str, whole: &str, offset: usize) -> Error {
    let body = body.trim();
    let number = code_header.map(str::to_owned).or_else(|| {
        body.strip_prefix("Code: ")
            .and_then(|rest| rest.split_once('.'))
            .map(|(n, _)| n.to_owned())
    });
    let core = body
        .split_once("DB::Exception: ")
        .map_or(body, |(_, rest)| rest);
    let core = core.split_once(" (version ").map_or(core, |(head, _)| head);
    // The name of the error, such as UNKNOWN_TABLE, ends the message in brackets.
    let (core, name) = match core.rfind(" (") {
        Some(at)
            if core.ends_with(')')
                && core[at + 2..core.len() - 1]
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b == b'_') =>
        {
            (&core[..at], Some(&core[at + 2..core.len() - 1]))
        }
        _ => (core, None),
    };
    // The list of what the parser would have accepted is long and says little.
    let message = core
        .split_once(". Expected one of:")
        .map_or(core, |(head, _)| head)
        .trim_end_matches('.')
        .to_owned();
    let position = message
        .split_once("failed at position ")
        .and_then(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<usize>().ok())
        .map(|p| {
            // ClickHouse counts bytes from 1 within the statement it was sent.
            let mut at = (offset + p - 1).min(whole.len());
            while !whole.is_char_boundary(at) {
                at -= 1;
            }
            whole[..at].chars().count() as u32 + 1
        });
    Error::Database {
        code: number.map(|n| match name {
            Some(name) => format!("{n} ({name})"),
            None => n,
        }),
        message: if message.is_empty() {
            body.to_owned()
        } else {
            message
        },
        position,
        line: None,
        hint: None,
        detail: None,
    }
}

/// The parts of a key expression list such as `id, toDate(ts)`, split at the
/// top level only.
pub fn key_parts(key: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in key.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(current.trim().to_owned());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    // A key written as a tuple, ORDER BY (a, b), is listed as its parts.
    if parts.len() == 1 && parts[0].starts_with('(') && parts[0].ends_with(')') {
        return key_parts(&parts[0][1..parts[0].len() - 1]);
    }
    parts
}

pub fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    match bytes {
        b if b >= KB * KB * KB => format!("{:.1} GB", b as f64 / (KB * KB * KB) as f64),
        b if b >= KB * KB => format!("{:.1} MB", b as f64 / (KB * KB) as f64),
        b => format!("{} kB", b.div_ceil(KB)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_values_by_type() {
        assert_eq!(
            to_value(&json!("18446744073709551615"), "UInt64"),
            Value::Decimal("18446744073709551615".into())
        );
        assert_eq!(to_value(&json!("-5"), "Int64"), Value::Int(-5));
        assert_eq!(to_value(&json!(7), "Nullable(UInt8)"), Value::Int(7));
        assert_eq!(to_value(&json!(null), "Nullable(String)"), Value::Null);
        assert_eq!(to_value(&json!(1.5), "Float64"), Value::Float(1.5));
        assert_eq!(
            to_value(&json!("12.345"), "Decimal(18, 3)"),
            Value::Decimal("12.345".into())
        );
        assert_eq!(to_value(&json!(true), "Bool"), Value::Bool(true));
        assert_eq!(
            to_value(&json!("2026-01-02 10:00:00.123"), "DateTime64(3, 'UTC')"),
            Value::Temporal("2026-01-02 10:00:00.123".into())
        );
        assert_eq!(
            to_value(&json!("x"), "LowCardinality(String)"),
            Value::Text("x".into())
        );
        assert_eq!(
            to_value(&json!([1, 2]), "Array(UInt8)"),
            Value::Json(json!([1, 2]))
        );
        assert_eq!(base_type("Nullable(LowCardinality(String))"), "String");
    }

    #[test]
    fn reads_server_errors() {
        let whole = "SELECT 1;\nSELEC ğ 1";
        let error = server_error(
            Some("62"),
            "Code: 62. DB::Exception: Syntax error: failed at position 1 (SELEC): SELEC ğ 1. Expected one of: Query, SELECT. (SYNTAX_ERROR) (version 25.8.33.6 (official build))",
            whole,
            10,
        );
        match error {
            Error::Database {
                code,
                message,
                position,
                ..
            } => {
                assert_eq!(code.as_deref(), Some("62 (SYNTAX_ERROR)"));
                assert_eq!(
                    message,
                    "Syntax error: failed at position 1 (SELEC): SELEC ğ 1"
                );
                assert_eq!(position, Some(11));
            }
            other => panic!("{other:?}"),
        }
        match server_error(
            None,
            "Code: 60. DB::Exception: Unknown table expression identifier 'nope'. (UNKNOWN_TABLE) (version 25.8)",
            "",
            0,
        ) {
            Error::Database {
                code,
                message,
                position,
                ..
            } => {
                assert_eq!(code.as_deref(), Some("60 (UNKNOWN_TABLE)"));
                assert_eq!(message, "Unknown table expression identifier 'nope'");
                assert_eq!(position, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn splits_keys() {
        assert_eq!(
            key_parts("id, toDate(ts, 'UTC')"),
            ["id", "toDate(ts, 'UTC')"]
        );
        assert_eq!(key_parts("(a, b)"), ["a", "b"]);
        assert!(key_parts("").is_empty());
        assert_eq!(size_text(1500), "2 kB");
    }
}
