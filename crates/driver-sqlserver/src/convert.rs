//! Values, type names and batches: the parts of the driver that need no server.

use dbclient_core::Value;
use tiberius::time::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use tiberius::{ColumnData, ColumnType, FromSql};

/// The type name of a result column, as T-SQL writes it.
pub fn type_name(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Null => "null",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 | ColumnType::Intn => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Float4 => "real",
        ColumnType::Float8 | ColumnType::Floatn => "float",
        ColumnType::Money => "money",
        ColumnType::Money4 => "smallmoney",
        ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::Decimaln | ColumnType::Numericn => "decimal",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::BigVarChar => "varchar",
        ColumnType::BigChar => "char",
        ColumnType::NVarchar => "nvarchar",
        ColumnType::NChar => "nchar",
        ColumnType::Text => "text",
        ColumnType::NText => "ntext",
        ColumnType::BigVarBin => "varbinary",
        ColumnType::BigBinary => "binary",
        ColumnType::Image => "image",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "udt",
        ColumnType::SSVariant => "sql_variant",
    }
}

/// One cell as a core value. `ty` decides how money is written.
pub fn to_value(data: &ColumnData<'static>, ty: ColumnType) -> Value {
    let temporal = |text: Option<String>| text.map_or(Value::Null, Value::Temporal);
    match data {
        ColumnData::U8(v) => v.map_or(Value::Null, |v| Value::Int(v.into())),
        ColumnData::I16(v) => v.map_or(Value::Null, |v| Value::Int(v.into())),
        ColumnData::I32(v) => v.map_or(Value::Null, |v| Value::Int(v.into())),
        ColumnData::I64(v) => v.map_or(Value::Null, Value::Int),
        // Money is a scaled integer on the server; four decimals show every digit it keeps.
        ColumnData::F64(Some(v)) if matches!(ty, ColumnType::Money | ColumnType::Money4) => {
            Value::Decimal(format!("{v:.4}"))
        }
        ColumnData::F32(v) => v.map_or(Value::Null, |v| Value::Float(v.into())),
        ColumnData::F64(v) => v.map_or(Value::Null, Value::Float),
        ColumnData::Bit(v) => v.map_or(Value::Null, Value::Bool),
        ColumnData::String(v) => v
            .as_ref()
            .map_or(Value::Null, |s| Value::Text(s.to_string())),
        // Written the way SQL Server itself shows it.
        ColumnData::Guid(v) => v.map_or(Value::Null, |g| {
            Value::Text(g.hyphenated().to_string().to_uppercase())
        }),
        ColumnData::Binary(v) => v.as_ref().map_or(Value::Null, |b| Value::Bytes(b.to_vec())),
        ColumnData::Numeric(v) => v.map_or(Value::Null, |n| {
            Value::Decimal(decimal_text(n.value(), n.scale()))
        }),
        ColumnData::Xml(v) => v
            .as_ref()
            .map_or(Value::Null, |x| Value::Text(x.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            temporal(
                NaiveDateTime::from_sql(data)
                    .ok()
                    .flatten()
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
            )
        }
        ColumnData::Date(_) => temporal(
            NaiveDate::from_sql(data)
                .ok()
                .flatten()
                .map(|d| d.format("%Y-%m-%d").to_string()),
        ),
        ColumnData::Time(_) => temporal(
            NaiveTime::from_sql(data)
                .ok()
                .flatten()
                .map(|t| t.format("%H:%M:%S%.f").to_string()),
        ),
        ColumnData::DateTimeOffset(_) => temporal(
            DateTime::<FixedOffset>::from_sql(data)
                .ok()
                .flatten()
                .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f%:z").to_string()),
        ),
    }
}

/// A scaled integer as decimal text: 12345 with scale 2 is `123.45`.
pub fn decimal_text(value: i128, scale: u8) -> String {
    let digits = value.unsigned_abs().to_string();
    let sign = if value < 0 { "-" } else { "" };
    let scale = scale as usize;
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    format!("{sign}{whole}.{fraction}")
}

/// A piece of the editor text that goes to the server on its own.
#[derive(Debug, PartialEq, Eq)]
pub struct Batch<'a> {
    pub text: &'a str,
    /// Lines before it in the whole text, so server line numbers can be made absolute.
    pub line_offset: u32,
}

/// Splits on `GO` lines, the batch separator of SQL Server's own tools. The
/// server does not know `GO`; sent as is it is a syntax error.
pub fn batches(text: &str) -> Vec<Batch<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut start_line = 0u32;
    let mut offset = 0;
    for (line_number, line) in text.split_inclusive('\n').enumerate() {
        let word = line.trim();
        let is_go = word.eq_ignore_ascii_case("go")
            || word
                .split_once(char::is_whitespace)
                .is_some_and(|(go, count)| {
                    go.eq_ignore_ascii_case("go")
                        && count.trim().bytes().all(|b| b.is_ascii_digit())
                });
        if is_go {
            push(&mut out, &text[start..offset], start_line);
            start = offset + line.len();
            start_line = line_number as u32 + 1;
        }
        offset += line.len();
    }
    push(&mut out, &text[start..], start_line);
    out
}

fn push<'a>(out: &mut Vec<Batch<'a>>, text: &'a str, line_offset: u32) {
    if !text.trim().is_empty() {
        out.push(Batch { text, line_offset });
    }
}

/// Whether the batch must stand alone: these statements take the rest of the
/// batch as their body, so nothing may be added after them.
pub fn stands_alone(batch: &str) -> bool {
    let words: Vec<String> = strip_leading_comments(batch)
        .split_whitespace()
        .take(4)
        .map(str::to_ascii_uppercase)
        .collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    // SET SHOWPLAN_* must be the only statement of its batch.
    if matches!(words.as_slice(), ["SET", option, ..] if option.starts_with("SHOWPLAN_")) {
        return true;
    }
    let rest = match words.as_slice() {
        ["CREATE", "OR", "ALTER", rest @ ..] | ["CREATE" | "ALTER", rest @ ..] => rest,
        _ => return false,
    };
    matches!(
        rest.first().copied(),
        Some(
            "PROC" | "PROCEDURE" | "VIEW" | "FUNCTION" | "TRIGGER" | "SCHEMA" | "DEFAULT" | "RULE"
        )
    )
}

fn strip_leading_comments(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if let Some(rest) = text.strip_prefix("--") {
            text = rest.split_once('\n').map_or("", |(_, after)| after);
        } else if let Some(rest) = text.strip_prefix("/*") {
            text = rest.split_once("*/").map_or("", |(_, after)| after);
        } else {
            return text;
        }
    }
}

/// `((0))` is how SQL Server stores the default `0`; the brackets carry nothing.
pub fn unwrap_default(text: &str) -> String {
    let mut text = text.trim();
    while text.starts_with('(') && text.ends_with(')') && balanced(&text[1..text.len() - 1]) {
        text = text[1..text.len() - 1].trim();
    }
    text.to_owned()
}

fn balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// A column's type as written in a table definition, from `sys.columns`.
pub fn column_type(name: &str, max_length: i16, precision: u8, scale: u8) -> String {
    match name {
        "varchar" | "char" | "varbinary" | "binary" => match max_length {
            -1 => format!("{name}(max)"),
            n => format!("{name}({n})"),
        },
        // Lengths are in bytes; these store two per character.
        "nvarchar" | "nchar" => match max_length {
            -1 => format!("{name}(max)"),
            n => format!("{name}({})", n / 2),
        },
        "decimal" | "numeric" => format!("{name}({precision},{scale})"),
        // Seven digits is the default and is left out, as SQL Server's tools do.
        "datetime2" | "time" | "datetimeoffset" if scale != 7 => format!("{name}({scale})"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_keep_every_digit() {
        assert_eq!(decimal_text(12345, 2), "123.45");
        assert_eq!(decimal_text(-5, 2), "-0.05");
        assert_eq!(decimal_text(7, 0), "7");
        assert_eq!(decimal_text(0, 3), "0.000");
        assert_eq!(
            decimal_text(123456789012345678901234567890123456, 6),
            "123456789012345678901234567890.123456"
        );
    }

    #[test]
    fn splits_on_go_lines_only() {
        let text = "CREATE TABLE t (a int)\nGO\nSELECT 'go'\n  go 2  \nSELECT 1\nGOTO x";
        let found = batches(text);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].text, "CREATE TABLE t (a int)\n");
        assert_eq!(found[1].text, "SELECT 'go'\n");
        assert_eq!(found[1].line_offset, 2);
        assert_eq!(found[2].text, "SELECT 1\nGOTO x");
        assert_eq!(found[2].line_offset, 4);
        assert!(batches("GO\n  \nGO").is_empty());
    }

    #[test]
    fn knows_statements_that_take_the_whole_batch() {
        assert!(stands_alone("CREATE PROCEDURE p AS SELECT 1"));
        assert!(stands_alone(
            "-- note\n/* x */ create or alter view v as select 1"
        ));
        assert!(stands_alone(
            "ALTER TRIGGER tr ON t AFTER INSERT AS SELECT 1"
        ));
        assert!(!stands_alone("CREATE TABLE t (a int)"));
        assert!(!stands_alone("SELECT 1"));
    }

    #[test]
    fn reads_stored_defaults_and_types() {
        assert_eq!(unwrap_default("((0))"), "0");
        assert_eq!(unwrap_default("(getdate())"), "getdate()");
        assert_eq!(unwrap_default("(N'a(b')"), "N'a(b'");
        assert_eq!(unwrap_default("((1)+(2))"), "(1)+(2)");
        assert_eq!(column_type("nvarchar", 100, 0, 0), "nvarchar(50)");
        assert_eq!(column_type("varbinary", -1, 0, 0), "varbinary(max)");
        assert_eq!(column_type("decimal", 9, 10, 2), "decimal(10,2)");
        assert_eq!(column_type("datetime2", 8, 27, 7), "datetime2");
        assert_eq!(column_type("datetime2", 6, 19, 0), "datetime2(0)");
        assert_eq!(column_type("int", 4, 10, 0), "int");
    }
}
