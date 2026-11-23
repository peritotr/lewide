//! Values, type names and errors: the parts of the driver that need no file.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta};
use dbclient_core::{Error, Value};
use duckdb::arrow::datatypes::DataType;
use duckdb::types::{TimeUnit, Value as Duck};
use serde_json::Value as Json;

/// The type of a result column, as DuckDB names it.
pub fn type_name(ty: &DataType) -> String {
    match ty {
        DataType::Null => "NULL".into(),
        DataType::Boolean => "BOOLEAN".into(),
        DataType::Int8 => "TINYINT".into(),
        DataType::Int16 => "SMALLINT".into(),
        DataType::Int32 => "INTEGER".into(),
        DataType::Int64 => "BIGINT".into(),
        DataType::UInt8 => "UTINYINT".into(),
        DataType::UInt16 => "USMALLINT".into(),
        DataType::UInt32 => "UINTEGER".into(),
        DataType::UInt64 => "UBIGINT".into(),
        DataType::Float16 | DataType::Float32 => "FLOAT".into(),
        DataType::Float64 => "DOUBLE".into(),
        DataType::Decimal128(p, s) | DataType::Decimal256(p, s) => format!("DECIMAL({p},{s})"),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "VARCHAR".into(),
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => "BLOB".into(),
        DataType::Date32 | DataType::Date64 => "DATE".into(),
        DataType::Time32(_) | DataType::Time64(_) => "TIME".into(),
        DataType::Timestamp(_, Some(_)) => "TIMESTAMP WITH TIME ZONE".into(),
        DataType::Timestamp(_, None) => "TIMESTAMP".into(),
        DataType::Interval(_) | DataType::Duration(_) => "INTERVAL".into(),
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_) => "LIST".into(),
        DataType::FixedSizeList(_, n) => format!("ARRAY[{n}]"),
        DataType::Struct(_) => "STRUCT".into(),
        DataType::Map(..) => "MAP".into(),
        DataType::Dictionary(..) => "ENUM".into(),
        DataType::Union(..) => "UNION".into(),
        other => format!("{other:?}").to_ascii_uppercase(),
    }
}

/// Whether the result column holds a timestamp with a time zone (always UTC here: ICU is not built in).
pub fn has_zone(ty: &DataType) -> bool {
    matches!(ty, DataType::Timestamp(_, Some(_)))
}

fn micros(unit: TimeUnit, value: i64) -> i64 {
    match unit {
        TimeUnit::Second => value.saturating_mul(1_000_000),
        TimeUnit::Millisecond => value.saturating_mul(1_000),
        TimeUnit::Microsecond => value,
        TimeUnit::Nanosecond => value / 1_000,
    }
}

fn timestamp_text(unit: TimeUnit, value: i64, zone: bool) -> String {
    let text = DateTime::from_timestamp_micros(micros(unit, value))
        .map(|d| d.naive_utc().format("%Y-%m-%d %H:%M:%S%.f").to_string())
        .unwrap_or_else(|| value.to_string());
    if zone { format!("{text}+00") } else { text }
}

fn date_text(days: i32) -> String {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .and_then(|epoch| epoch.checked_add_signed(TimeDelta::days(days.into())))
        .map_or_else(|| days.to_string(), |d| d.format("%Y-%m-%d").to_string())
}

fn time_text(unit: TimeUnit, value: i64) -> String {
    let micros = micros(unit, value);
    if micros == 86_400_000_000 {
        // DuckDB allows 24:00:00.
        return "24:00:00".into();
    }
    NaiveTime::from_num_seconds_from_midnight_opt(
        (micros / 1_000_000) as u32,
        ((micros % 1_000_000) * 1_000) as u32,
    )
    .map_or_else(
        || micros.to_string(),
        |t| t.format("%H:%M:%S%.f").to_string(),
    )
}

/// `1 year 2 months 3 days 04:05:06`, as DuckDB writes an interval.
pub fn interval_text(months: i32, days: i32, nanos: i64) -> String {
    let mut parts = Vec::new();
    let (years, months) = (months / 12, months % 12);
    let unit = |n: i64, one: &str| format!("{n} {one}{}", if n.abs() == 1 { "" } else { "s" });
    if years != 0 {
        parts.push(unit(years.into(), "year"));
    }
    if months != 0 {
        parts.push(unit(months.into(), "month"));
    }
    if days != 0 {
        parts.push(unit(days.into(), "day"));
    }
    if nanos != 0 || parts.is_empty() {
        let sign = if nanos < 0 { "-" } else { "" };
        let total = (nanos / 1_000).unsigned_abs();
        let (h, rest) = (total / 3_600_000_000, total % 3_600_000_000);
        let (m, rest) = (rest / 60_000_000, rest % 60_000_000);
        let (s, us) = (rest / 1_000_000, rest % 1_000_000);
        let fraction = if us == 0 {
            String::new()
        } else {
            format!(".{us:06}").trim_end_matches('0').to_owned()
        };
        parts.push(format!("{sign}{h:02}:{m:02}:{s:02}{fraction}"));
    }
    parts.join(" ")
}

/// One cell as a core value. `zone` says the column is a timestamp with a time zone.
pub fn to_value(value: Duck, zone: bool) -> Value {
    match value {
        Duck::Null => Value::Null,
        Duck::Boolean(b) => Value::Bool(b),
        Duck::TinyInt(n) => Value::Int(n.into()),
        Duck::SmallInt(n) => Value::Int(n.into()),
        Duck::Int(n) => Value::Int(n.into()),
        Duck::BigInt(n) => Value::Int(n),
        Duck::UTinyInt(n) => Value::Int(n.into()),
        Duck::USmallInt(n) => Value::Int(n.into()),
        Duck::UInt(n) => Value::Int(n.into()),
        Duck::UBigInt(n) => {
            i64::try_from(n).map_or_else(|_| Value::Decimal(n.to_string()), Value::Int)
        }
        Duck::HugeInt(n) => {
            i64::try_from(n).map_or_else(|_| Value::Decimal(n.to_string()), Value::Int)
        }
        Duck::UHugeInt(n) => {
            i64::try_from(n).map_or_else(|_| Value::Decimal(n.to_string()), Value::Int)
        }
        Duck::Float(f) => Value::Float(f.into()),
        Duck::Double(f) => Value::Float(f),
        Duck::Decimal(d) => Value::Decimal(d.to_string()),
        Duck::Text(s) => Value::Text(s),
        Duck::Enum(s) => Value::Text(s),
        Duck::Blob(b) | Duck::Geometry(b) => Value::Bytes(b),
        Duck::Timestamp(unit, v) => Value::Temporal(timestamp_text(unit, v, zone)),
        Duck::Date32(days) => Value::Temporal(date_text(days)),
        Duck::Time64(unit, v) => Value::Temporal(time_text(unit, v)),
        Duck::Interval {
            months,
            days,
            nanos,
        } => Value::Text(interval_text(months, days, nanos)),
        nested => Value::Json(to_json(nested)),
    }
}

/// Lists, structs and maps as JSON, their scalars as JSON scalars.
fn to_json(value: Duck) -> Json {
    match value {
        Duck::List(items) | Duck::Array(items) => Json::Array(items.into_iter().map(to_json).collect()),
        Duck::Struct(fields) => Json::Object(fields.iter().map(|(k, v)| (k.clone(), to_json(v.clone()))).collect()),
        // Map keys need not be text, so a map is a list of {key, value} pairs.
        Duck::Map(entries) => Json::Array(
            entries
                .iter()
                .map(|(k, v)| serde_json::json!({ "key": to_json(k.clone()), "value": to_json(v.clone()) }))
                .collect(),
        ),
        Duck::Union(inner) => to_json(*inner),
        scalar => match to_value(scalar, false) {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(b),
            Value::Int(n) => Json::from(n),
            Value::Float(f) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
            Value::Bytes(b) => Json::String(b.iter().map(|x| format!("{x:02x}")).collect()),
            Value::Json(j) => j,
            Value::Decimal(s) | Value::Text(s) | Value::Temporal(s) => Json::String(s),
        },
    }
}

/// A DuckDB error: `Parser Error: syntax error at or near "SELEC"` with
/// `LINE 1: SELEC 1` and a caret under the place. The kind of error is kept as
/// the code; the line and column are read from the caret, so the editor can
/// point at it. `offset` is the statement's start in `whole`, in bytes.
pub fn database_error(text: &str, whole: &str, offset: usize) -> Error {
    let (head, tail) = text
        .split_once("\n\nLINE ")
        .map_or((text, None), |(h, t)| (h, Some(t)));
    let (code, message) = match head.split_once(" Error: ") {
        Some((kind, rest)) if !kind.contains('\n') => {
            (Some(format!("{kind} Error")), rest.trim().to_owned())
        }
        _ => (None, head.trim().to_owned()),
    };
    let mut position = None;
    let mut line = None;
    if let Some(tail) = tail {
        let mut lines = tail.lines();
        let shown = lines.next().unwrap_or("");
        let caret = lines.next().and_then(|l| l.find('^'));
        if let Some((number, rest)) = shown.split_once(": ")
            && let Ok(number) = number.trim().parse::<usize>()
        {
            // The caret sits under "LINE n: …", counted in characters.
            let prefix = "LINE ".len() + shown[..shown.len() - rest.len()].chars().count();
            // Only when the line is shown whole can the caret be counted back to a column.
            if let Some(col) = caret.filter(|c| *c >= prefix && !rest.starts_with("...")) {
                let statement = &whole[offset.min(whole.len())..];
                let line_start: usize = statement
                    .split_inclusive('\n')
                    .take(number - 1)
                    .map(str::len)
                    .sum();
                let before = whole[..(offset + line_start).min(whole.len())]
                    .chars()
                    .count();
                position = Some((before + (col - prefix)) as u32 + 1);
            } else {
                let before = whole[..offset.min(whole.len())].matches('\n').count();
                line = Some((before + number) as u32);
            }
        }
    }
    Error::Database {
        code,
        message,
        position,
        line,
        hint: None,
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_times_and_numbers() {
        assert_eq!(
            timestamp_text(TimeUnit::Microsecond, 1_767_348_000_123_456, false),
            "2026-01-02 10:00:00.123456"
        );
        assert_eq!(
            timestamp_text(TimeUnit::Microsecond, 0, true),
            "1970-01-01 00:00:00+00"
        );
        assert_eq!(date_text(20455), "2026-01-02");
        assert_eq!(
            time_text(TimeUnit::Microsecond, 37_230_500_000),
            "10:20:30.500"
        );
        assert_eq!(
            interval_text(14, 3, 3_661_000_000_000),
            "1 year 2 months 3 days 01:01:01"
        );
        assert_eq!(interval_text(0, 0, 0), "00:00:00");
        assert_eq!(interval_text(0, 1, 0), "1 day");
        assert_eq!(
            to_value(Duck::HugeInt(i128::MAX), false),
            Value::Decimal(i128::MAX.to_string())
        );
        assert_eq!(
            to_value(
                Duck::List(vec![Duck::Int(1), Duck::Text("a".into())]),
                false
            ),
            Value::Json(serde_json::json!([1, "a"]))
        );
    }

    #[test]
    fn reads_errors() {
        let whole = "SELECT 1;\nSELEC ğ 1";
        match database_error(
            "Parser Error: syntax error at or near \"SELEC\"\n\nLINE 1: SELEC ğ 1\n        ^",
            whole,
            10,
        ) {
            Error::Database {
                code,
                message,
                position,
                ..
            } => {
                assert_eq!(code.as_deref(), Some("Parser Error"));
                assert_eq!(message, "syntax error at or near \"SELEC\"");
                assert_eq!(position, Some(11));
            }
            other => panic!("{other:?}"),
        }
        match database_error(
            "Catalog Error: Table with name nope does not exist!\nDid you mean \"note\"?",
            "",
            0,
        ) {
            Error::Database {
                code,
                message,
                position,
                ..
            } => {
                assert_eq!(code.as_deref(), Some("Catalog Error"));
                assert!(message.starts_with("Table with name nope"));
                assert_eq!(position, None);
            }
            other => panic!("{other:?}"),
        }
    }
}
