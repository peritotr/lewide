//! Writing values into SQL text, per engine.
//!
//! Row edits are sent as statements with literal values. Every value goes
//! through [`literal`], which either produces a literal that the engine reads
//! back as exactly that value, or refuses.

use serde::{Deserialize, Serialize};

use crate::{Error, Result, Value};

/// The SQL flavours whose literal syntax differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlDialect {
    Postgres,
    #[serde(alias = "mysql")]
    MySql,
    Sqlite,
    /// Microsoft SQL Server (T-SQL).
    #[serde(alias = "sqlserver")]
    SqlServer,
    /// ClickHouse: backslash escapes in strings and quoted names.
    #[serde(alias = "clickhouse")]
    ClickHouse,
    /// DuckDB: PostgreSQL's syntax, but a backslash in a string is plain text.
    #[serde(alias = "duckdb")]
    DuckDb,
}

impl SqlDialect {
    /// Quotes a name so it can be used as an identifier.
    pub fn quote_ident(self, name: &str) -> String {
        match self {
            SqlDialect::MySql => format!("`{}`", name.replace('`', "``")),
            // Brackets mean the same whatever QUOTED_IDENTIFIER is set to.
            SqlDialect::SqlServer => format!("[{}]", name.replace(']', "]]")),
            SqlDialect::ClickHouse => format!("`{}`", backslashed(name, '`')),
            SqlDialect::Postgres | SqlDialect::Sqlite | SqlDialect::DuckDb => {
                format!("\"{}\"", name.replace('"', "\"\""))
            }
        }
    }

    /// The statement that opens a transaction: a bare `BEGIN` starts a block in T-SQL.
    pub fn begin(self) -> &'static str {
        match self {
            SqlDialect::SqlServer => "BEGIN TRANSACTION",
            _ => "BEGIN",
        }
    }
}

/// The value as a literal for `dialect`.
pub fn literal(dialect: SqlDialect, value: &Value) -> Result<String> {
    Ok(match value {
        Value::Null => "NULL".to_owned(),
        // Neither has a boolean literal; their bit and integer columns read 1 and 0.
        Value::Bool(true) if matches!(dialect, SqlDialect::Sqlite | SqlDialect::SqlServer) => {
            "1".to_owned()
        }
        Value::Bool(false) if matches!(dialect, SqlDialect::Sqlite | SqlDialect::SqlServer) => {
            "0".to_owned()
        }
        Value::Bool(true) => "TRUE".to_owned(),
        Value::Bool(false) => "FALSE".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_finite() => format!("{f:?}"),
        Value::Float(_) => {
            return Err(Error::Query(
                "NaN and infinity cannot be written as a value".into(),
            ));
        }
        // Arbitrary text is never placed unquoted: only a plain number passes.
        Value::Decimal(text) if is_number(text) => text.clone(),
        Value::Decimal(text) => return Err(Error::Query(format!("not a number: {text}"))),
        Value::Text(text) | Value::Temporal(text) => quote(dialect, text)?,
        Value::Json(json) => quote(dialect, &json.to_string())?,
        Value::Bytes(bytes) => {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            match dialect {
                // Read as bytea through the column's type.
                SqlDialect::Postgres => format!("'\\x{hex}'"),
                SqlDialect::MySql | SqlDialect::Sqlite => format!("X'{hex}'"),
                SqlDialect::SqlServer => format!("0x{hex}"),
                // Its strings hold any bytes.
                SqlDialect::ClickHouse => format!("unhex('{hex}')"),
                SqlDialect::DuckDb => format!("from_hex('{hex}')"),
            }
        }
    })
}

/// A quoted string that means the same whatever the session's escape settings are.
fn quote(dialect: SqlDialect, text: &str) -> Result<String> {
    let doubled = text.replace('\'', "''");
    Ok(match dialect {
        SqlDialect::Sqlite | SqlDialect::DuckDb => format!("'{doubled}'"),
        // N'' keeps characters outside the database's code page; a backslash is plain text.
        SqlDialect::SqlServer => format!("N'{doubled}'"),
        // A backslash always escapes in ClickHouse.
        SqlDialect::ClickHouse => format!("'{}'", backslashed(text, '\'')),
        SqlDialect::Postgres => {
            if text.contains('\0') {
                return Err(Error::Query(
                    "PostgreSQL text cannot contain a NUL character".into(),
                ));
            }
            if text.contains('\\') {
                // With E'' a backslash is always an escape, whatever standard_conforming_strings says.
                format!("E'{}'", doubled.replace('\\', "\\\\"))
            } else {
                format!("'{doubled}'")
            }
        }
        SqlDialect::MySql => {
            if text.contains('\\') {
                // Whether a backslash escapes depends on sql_mode; hex has one meaning.
                let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
                format!("CONVERT(X'{hex}' USING utf8mb4)")
            } else {
                format!("'{doubled}'")
            }
        }
    })
}

/// `text` with backslashes and `quote` escaped by a backslash.
fn backslashed(text: &str, quote: char) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        if c == '\\' || c == quote {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Digits with an optional sign, fraction and exponent, and nothing else.
fn is_number(text: &str) -> bool {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (unsigned, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = match mantissa.split_once('.') {
        Some((whole, fraction)) => (digits(whole) || whole.is_empty()) && digits(fraction),
        None => digits(mantissa),
    };
    mantissa_ok && exponent.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use SqlDialect::*;

    #[test]
    fn plain_values() {
        assert_eq!(literal(Postgres, &Value::Null).unwrap(), "NULL");
        assert_eq!(literal(Postgres, &Value::Bool(true)).unwrap(), "TRUE");
        assert_eq!(literal(Sqlite, &Value::Bool(false)).unwrap(), "0");
        assert_eq!(literal(MySql, &Value::Int(-7)).unwrap(), "-7");
        assert_eq!(literal(MySql, &Value::Float(1.5)).unwrap(), "1.5");
        assert_eq!(
            literal(Postgres, &Value::Decimal("12345.678901234567890123".into())).unwrap(),
            "12345.678901234567890123"
        );
        assert_eq!(
            literal(Postgres, &Value::Bytes(vec![0, 255])).unwrap(),
            "'\\x00ff'"
        );
        assert_eq!(
            literal(MySql, &Value::Bytes(vec![0, 255])).unwrap(),
            "X'00ff'"
        );
        assert_eq!(
            literal(Sqlite, &Value::Json(serde_json::json!({"a": "it's"}))).unwrap(),
            r#"'{"a":"it''s"}'"#
        );
    }

    #[test]
    fn text_cannot_break_out_of_its_quotes() {
        let hostile = "x'; DROP TABLE users; --";
        for dialect in [Postgres, MySql, Sqlite] {
            assert_eq!(
                literal(dialect, &Value::Text(hostile.into())).unwrap(),
                "'x''; DROP TABLE users; --'"
            );
        }
        // A backslash before a quote is the classic way out; each engine gets a form where it is inert.
        let backslash = Value::Text("a\\' OR 1=1 --".into());
        assert_eq!(
            literal(Postgres, &backslash).unwrap(),
            "E'a\\\\'' OR 1=1 --'"
        );
        assert!(
            literal(MySql, &backslash)
                .unwrap()
                .starts_with("CONVERT(X'615c27")
        );
        assert_eq!(literal(Sqlite, &backslash).unwrap(), "'a\\'' OR 1=1 --'");
        assert!(literal(Postgres, &Value::Text("a\0b".into())).is_err());
    }

    #[test]
    fn only_numbers_go_unquoted() {
        for good in ["0", "-12", "+3.50", ".5", "1e10", "2.5E-3"] {
            assert!(
                literal(Postgres, &Value::Decimal(good.into())).is_ok(),
                "{good}"
            );
        }
        for bad in [
            "",
            "1; DROP TABLE t",
            "1 OR 1=1",
            "0x10",
            "1.",
            "--1",
            "1e",
            "NaN",
        ] {
            assert!(
                literal(Postgres, &Value::Decimal(bad.into())).is_err(),
                "{bad}"
            );
        }
        assert!(literal(MySql, &Value::Float(f64::NAN)).is_err());
    }

    #[test]
    fn duckdb_forms() {
        assert_eq!(DuckDb.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(
            literal(DuckDb, &Value::Text("x'\\; DROP TABLE t; --".into())).unwrap(),
            "'x''\\; DROP TABLE t; --'"
        );
        assert_eq!(
            literal(DuckDb, &Value::Bytes(vec![0, 255])).unwrap(),
            "from_hex('00ff')"
        );
        assert_eq!(literal(DuckDb, &Value::Bool(false)).unwrap(), "FALSE");
        assert_eq!(
            serde_json::from_str::<SqlDialect>("\"duckdb\"").unwrap(),
            DuckDb
        );
    }

    #[test]
    fn clickhouse_forms() {
        assert_eq!(ClickHouse.quote_ident("a`b\\c"), "`a\\`b\\\\c`");
        assert_eq!(
            literal(ClickHouse, &Value::Text("x'\\'; DROP TABLE t; --".into())).unwrap(),
            "'x\\'\\\\\\'; DROP TABLE t; --'"
        );
        assert_eq!(literal(ClickHouse, &Value::Bool(true)).unwrap(), "TRUE");
        assert_eq!(
            literal(ClickHouse, &Value::Bytes(vec![0, 255])).unwrap(),
            "unhex('00ff')"
        );
        assert_eq!(
            serde_json::from_str::<SqlDialect>("\"clickhouse\"").unwrap(),
            ClickHouse
        );
    }

    #[test]
    fn sql_server_forms() {
        assert_eq!(SqlServer.quote_ident("a]b"), "[a]]b]");
        assert_eq!(SqlServer.begin(), "BEGIN TRANSACTION");
        assert_eq!(Postgres.begin(), "BEGIN");
        assert_eq!(literal(SqlServer, &Value::Bool(true)).unwrap(), "1");
        assert_eq!(
            literal(SqlServer, &Value::Bytes(vec![0, 255])).unwrap(),
            "0x00ff"
        );
        assert_eq!(
            literal(SqlServer, &Value::Text("x'; DROP TABLE t; --".into())).unwrap(),
            "N'x''; DROP TABLE t; --'"
        );
        assert_eq!(
            literal(SqlServer, &Value::Text("a\\' OR 1=1".into())).unwrap(),
            "N'a\\'' OR 1=1'"
        );
        assert_eq!(
            serde_json::from_str::<SqlDialect>("\"sqlserver\"").unwrap(),
            SqlServer
        );
    }
}
