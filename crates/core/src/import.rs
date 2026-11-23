//! Reading rows from a file and turning them into values of a table's columns.
//!
//! The whole file is read into memory; [`MAX_FILE_BYTES`] bounds that. Text is
//! UTF-8, with or without a byte-order mark.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{ColumnDescription, Error, Result, Value};

pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;

fn default_delimiter() -> char {
    ','
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum ImportFormat {
    Csv {
        #[serde(default = "default_delimiter")]
        delimiter: char,
        /// The first row holds the column names.
        #[serde(default = "yes")]
        header: bool,
        /// Spaces around unquoted values are dropped.
        #[serde(default)]
        trim: bool,
        /// Unquoted values read as NULL, besides the empty one.
        #[serde(default)]
        null_tokens: Vec<String>,
    },
    /// One array of objects.
    Json,
    /// One object per line.
    JsonLines,
    /// An Excel or OpenDocument spreadsheet (`.xlsx`, `.xlsm`, `.xls`, `.ods`).
    Xlsx {
        /// The sheet to read; the first when none is given.
        #[serde(default)]
        sheet: Option<String>,
        /// The first row holds the column names.
        #[serde(default = "yes")]
        header: bool,
    },
}

/// A file as a table of texts. `None` is a missing value.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// The line each row starts on in the file, 1-based.
    pub lines: Vec<u32>,
}

/// What the first step of the wizard shows about a file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Inspection {
    pub columns: Vec<String>,
    /// The first rows, as read.
    pub sample: Vec<Vec<Option<String>>>,
    pub total_rows: usize,
    pub bytes: u64,
    /// The workbook's sheets, for a spreadsheet; empty for text files.
    pub sheets: Vec<String>,
}

pub fn read_file(path: &Path, format: &ImportFormat) -> Result<Source> {
    let size = std::fs::metadata(path).map_err(file_err)?.len();
    if size > MAX_FILE_BYTES {
        return Err(Error::File(format!(
            "the file is {} MB; files over {} MB cannot be imported yet",
            size / 1024 / 1024,
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    if let ImportFormat::Xlsx { sheet, header } = format {
        return crate::xlsx::read(path, sheet.as_deref(), *header);
    }
    let bytes = std::fs::read(path).map_err(file_err)?;
    let text =
        String::from_utf8(bytes).map_err(|_| Error::File("the file is not UTF-8 text".into()))?;
    parse(text.strip_prefix('\u{feff}').unwrap_or(&text), format)
}

fn file_err(error: std::io::Error) -> Error {
    Error::File(error.to_string())
}

pub fn parse(text: &str, format: &ImportFormat) -> Result<Source> {
    match format {
        ImportFormat::Csv {
            delimiter,
            header,
            trim,
            null_tokens,
        } => parse_csv(text, *delimiter, *header, *trim, null_tokens),
        ImportFormat::Json => {
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| Error::File(format!("not valid JSON: {e}")))?;
            let serde_json::Value::Array(items) = value else {
                return Err(Error::File(
                    "the JSON file must hold one array of objects".into(),
                ));
            };
            from_objects(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (i as u32 + 1, v)),
            )
        }
        ImportFormat::Xlsx { .. } => Err(Error::File(
            "a spreadsheet is read from its file, not from text".into(),
        )),
        ImportFormat::JsonLines => {
            let mut items = Vec::new();
            for (i, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value = serde_json::from_str(line)
                    .map_err(|e| Error::File(format!("line {} is not valid JSON: {e}", i + 1)))?;
                items.push((i as u32 + 1, value));
            }
            from_objects(items.into_iter())
        }
    }
}

fn from_objects(items: impl Iterator<Item = (u32, serde_json::Value)>) -> Result<Source> {
    let mut columns: Vec<String> = Vec::new();
    let mut objects = Vec::new();
    for (line, item) in items {
        let serde_json::Value::Object(object) = item else {
            return Err(Error::File(format!("item {line} is not an object")));
        };
        for key in object.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
        objects.push((line, object));
    }
    let mut source = Source {
        columns,
        rows: Vec::with_capacity(objects.len()),
        lines: Vec::with_capacity(objects.len()),
    };
    for (line, object) in objects {
        source.rows.push(
            source
                .columns
                .iter()
                .map(|column| match object.get(column) {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(text)) => Some(text.clone()),
                    Some(other) => Some(other.to_string()),
                })
                .collect(),
        );
        source.lines.push(line);
    }
    Ok(source)
}

fn parse_csv(
    text: &str,
    delimiter: char,
    header: bool,
    trim: bool,
    null_tokens: &[String],
) -> Result<Source> {
    if matches!(delimiter, '"' | '\r' | '\n') {
        return Err(Error::InvalidConfig(
            "the delimiter cannot be a quote or a line break".into(),
        ));
    }
    // Each field with whether it was quoted: only unquoted text can mean NULL.
    let mut records: Vec<(u32, Vec<(String, bool)>)> = Vec::new();
    let mut record: Vec<(String, bool)> = Vec::new();
    let mut field = String::new();
    let (mut quoted, mut in_quotes, mut line, mut start) = (false, false, 1u32, 1u32);
    let mut chars = text.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                _ => {
                    if c == '\n' {
                        line += 1;
                    }
                    field.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if field.trim().is_empty() && !quoted => {
                field.clear();
                quoted = true;
                in_quotes = true;
                pending = true;
            }
            c if c == delimiter => {
                record.push((std::mem::take(&mut field), quoted));
                quoted = false;
                pending = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                if pending || !field.is_empty() {
                    record.push((std::mem::take(&mut field), quoted));
                    records.push((start, std::mem::take(&mut record)));
                }
                quoted = false;
                pending = false;
                line += 1;
                start = line;
            }
            _ => {
                field.push(c);
                pending = true;
            }
        }
    }
    if in_quotes {
        return Err(Error::File(format!(
            "a quoted value that starts on line {start} is never closed"
        )));
    }
    if pending || !field.is_empty() {
        record.push((field, quoted));
        records.push((start, record));
    }

    let mut records = records.into_iter();
    let width;
    let columns: Vec<String> = if header {
        let Some((_, names)) = records.next() else {
            return Err(Error::File("the file is empty".into()));
        };
        width = names.len();
        names
            .into_iter()
            .map(|(name, _)| name.trim().to_owned())
            .collect()
    } else {
        let all: Vec<_> = records.collect();
        width = all.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
        records = all.into_iter();
        (1..=width).map(|i| format!("column {i}")).collect()
    };

    let mut source = Source {
        columns,
        rows: Vec::new(),
        lines: Vec::new(),
    };
    for (line, record) in records {
        if record.len() > width {
            return Err(Error::File(format!(
                "line {line} has {} values, but there are {width} columns",
                record.len()
            )));
        }
        let mut row: Vec<Option<String>> = record
            .into_iter()
            .map(|(text, quoted)| {
                if quoted {
                    return Some(text);
                }
                let text = if trim { text.trim().to_owned() } else { text };
                (!text.is_empty() && !null_tokens.contains(&text)).then_some(text)
            })
            .collect();
        // A short line leaves its last columns empty.
        row.resize(width, None);
        source.rows.push(row);
        source.lines.push(line);
    }
    Ok(source)
}

pub fn inspect(path: &Path, format: &ImportFormat) -> Result<Inspection> {
    let bytes = std::fs::metadata(path).map_err(file_err)?.len();
    let source = read_file(path, format)?;
    let sheets = match format {
        ImportFormat::Xlsx { .. } => crate::xlsx::sheet_names(path)?,
        _ => Vec::new(),
    };
    Ok(Inspection {
        sheets,
        total_rows: source.rows.len(),
        sample: source.rows.into_iter().take(20).collect(),
        columns: source.columns,
        bytes,
    })
}

/// The kind of value a column takes, from its type name.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Int,
    Decimal,
    Float,
    Bool,
    Json,
    Temporal,
    Bytes,
    Text,
}

pub fn family(type_name: Option<&str>) -> Family {
    let mut t = type_name.unwrap_or("").trim().to_ascii_lowercase();
    // ClickHouse wraps the type: Nullable(UInt64), LowCardinality(String).
    while let Some(inner) = ["nullable(", "lowcardinality("]
        .iter()
        .find_map(|w| t.strip_prefix(w).and_then(|rest| rest.strip_suffix(')')))
    {
        t = inner.trim().to_owned();
    }
    let starts = |names: &[&str]| {
        names
            .iter()
            .any(|n| t == *n || t.starts_with(&format!("{n}(")) || t.starts_with(&format!("{n} ")))
    };
    // SQL Server's bit is its boolean; PostgreSQL's bit(n) is a bit string and stays text.
    if starts(&["bool", "boolean"]) || t == "tinyint(1)" || t == "bit" {
        Family::Bool
    } else if starts(&[
        "tinyint",
        "smallint",
        "mediumint",
        "int",
        "integer",
        "bigint",
        "int2",
        "int4",
        "int8",
        "serial",
        "smallserial",
        "bigserial",
        // ClickHouse.
        "int16",
        "int32",
        "int64",
        "int128",
        "int256",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uint128",
        "uint256",
    ]) {
        Family::Int
    } else if starts(&["numeric", "decimal", "money", "smallmoney", "number"]) {
        Family::Decimal
    } else if starts(&[
        "real", "float", "float4", "float8", "double", "float32", "float64", "bfloat16",
    ]) {
        Family::Float
    } else if starts(&["json", "jsonb"]) {
        Family::Json
    } else if starts(&[
        "date",
        "time",
        "timestamp",
        "datetime",
        "timetz",
        "timestamptz",
        "interval",
        "year",
        "datetime2",
        "datetimeoffset",
        "smalldatetime",
        "date32",
        "datetime64",
    ]) {
        Family::Temporal
    } else if starts(&[
        "bytea",
        "blob",
        "tinyblob",
        "mediumblob",
        "longblob",
        "binary",
        "varbinary",
        "image",
    ]) {
        Family::Bytes
    } else {
        Family::Text
    }
}

/// A text from a file as a value of the column, or why it cannot be one.
pub fn convert(
    text: Option<&str>,
    column: &ColumnDescription,
) -> std::result::Result<Value, String> {
    let Some(text) = text else {
        // A missing value takes the column's default when it has one.
        return if column.nullable || column.default_value.is_some() {
            Ok(Value::Null)
        } else {
            Err("is required (NOT NULL, no default)".into())
        };
    };
    let trimmed = text.trim();
    let type_name = column.type_name.as_deref().unwrap_or("text");
    match family(column.type_name.as_deref()) {
        Family::Int => trimmed
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| format!("is not a whole number ({type_name})")),
        Family::Decimal => {
            let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
            let ok = !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
                && digits.matches('.').count() <= 1
                && digits != ".";
            ok.then(|| Value::Decimal(trimmed.trim_start_matches('+').to_owned()))
                .ok_or_else(|| format!("is not a number ({type_name})"))
        }
        Family::Float => trimmed
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(Value::Float)
            .ok_or_else(|| format!("is not a number ({type_name})")),
        Family::Bool => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "1" => Ok(Value::Bool(true)),
            "false" | "f" | "no" | "n" | "0" => Ok(Value::Bool(false)),
            _ => Err("is not true or false".into()),
        },
        Family::Json => serde_json::from_str(text)
            .map(Value::Json)
            .map_err(|e| format!("is not valid JSON: {e}")),
        Family::Temporal if trimmed.is_empty() => Err("is empty".into()),
        Family::Temporal => Ok(Value::Temporal(trimmed.to_owned())),
        Family::Bytes => Err("binary columns cannot be imported from text yet".into()),
        Family::Text => Ok(Value::Text(text.to_owned())),
    }
}

/// Which file column feeds which table column.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Mapping {
    /// Index of the column in the file.
    pub source: usize,
    /// Name of the column in the table.
    pub column: String,
}

/// A row that cannot be written, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowError {
    /// The line the row starts on in the file.
    pub line: u32,
    pub column: Option<String>,
    pub value: Option<String>,
    pub reason: String,
}

/// One file row as the values the mapped columns would get.
pub fn convert_row(
    row: &[Option<String>],
    line: u32,
    mapping: &[Mapping],
    table: &[ColumnDescription],
) -> std::result::Result<Vec<crate::edit::Cell>, RowError> {
    mapping
        .iter()
        .map(|m| {
            let column = table
                .iter()
                .find(|c| c.name == m.column)
                .ok_or_else(|| RowError {
                    line,
                    column: Some(m.column.clone()),
                    value: None,
                    reason: "no such column in the table".into(),
                })?;
            let text = row.get(m.source).and_then(|v| v.as_deref());
            convert(text, column)
                .map(|value| crate::edit::Cell {
                    column: column.name.clone(),
                    value,
                })
                .map_err(|reason| RowError {
                    line,
                    column: Some(column.name.clone()),
                    value: text.map(str::to_owned),
                    reason,
                })
        })
        .collect()
}

/// Table columns that would be left to their default or to NULL, and those
/// that must have a value but get none from the file.
/// `implicit_defaults` is for engines that fill any column left out with its
/// type's default (ClickHouse), where no column is required.
pub fn check_mapping(
    mapping: &[Mapping],
    table: &[ColumnDescription],
    implicit_defaults: bool,
) -> std::result::Result<Vec<String>, String> {
    let mut missing = Vec::new();
    for column in table {
        if mapping.iter().any(|m| m.column == column.name) {
            continue;
        }
        if !implicit_defaults && !column.nullable && column.default_value.is_none() {
            return Err(format!(
                "{} is required (NOT NULL, no default) and no file column is mapped to it",
                column.name
            ));
        }
        missing.push(column.name.clone());
    }
    for (i, m) in mapping.iter().enumerate() {
        if mapping[..i].iter().any(|o| o.column == m.column) {
            return Err(format!("two file columns are mapped to {}", m.column));
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(text: &str, header: bool) -> Source {
        parse(
            text,
            &ImportFormat::Csv {
                delimiter: ',',
                header,
                trim: true,
                null_tokens: vec!["NULL".into()],
            },
        )
        .unwrap()
    }

    fn column(name: &str, type_name: &str, nullable: bool) -> ColumnDescription {
        ColumnDescription {
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable,
            default_value: None,
            primary_key: false,
            comment: None,
            enum_values: None,
        }
    }

    #[test]
    fn reads_quoted_values_line_breaks_and_nulls() {
        let source = csv(
            "id, name ,note\r\n1, Ada ,\"says \"\"hi\"\", twice\"\n2,NULL,\"two\nlines\"\n3,\"NULL\",\"\"\n\n4,x\n",
            true,
        );
        assert_eq!(source.columns, ["id", "name", "note"]);
        assert_eq!(
            source.rows,
            vec![
                vec![
                    Some("1".into()),
                    Some("Ada".into()),
                    Some("says \"hi\", twice".into())
                ],
                vec![Some("2".into()), None, Some("two\nlines".into())],
                // Quoted: the word NULL and an empty text, not missing values.
                vec![Some("3".into()), Some("NULL".into()), Some(String::new())],
                // A short row leaves the rest empty; the blank line is not a row.
                vec![Some("4".into()), Some("x".into()), None],
            ]
        );
        assert_eq!(source.lines, [2, 3, 5, 7]);
    }

    #[test]
    fn names_columns_when_there_is_no_header_and_rejects_broken_files() {
        let source = csv("1,2\n3,4,5\n", false);
        assert_eq!(source.columns, ["column 1", "column 2", "column 3"]);
        assert_eq!(
            source.rows[0],
            vec![Some("1".into()), Some("2".into()), None]
        );
        assert!(
            parse(
                "a,b\n1,2,3\n",
                &ImportFormat::Csv {
                    delimiter: ',',
                    header: true,
                    trim: false,
                    null_tokens: vec![]
                }
            )
            .is_err()
        );
        assert!(
            parse(
                "a\n\"open\n",
                &ImportFormat::Csv {
                    delimiter: ',',
                    header: true,
                    trim: false,
                    null_tokens: vec![]
                }
            )
            .is_err()
        );
        let tabs = parse(
            "a\tb\n1\t2\n",
            &ImportFormat::Csv {
                delimiter: '\t',
                header: true,
                trim: false,
                null_tokens: vec![],
            },
        )
        .unwrap();
        assert_eq!(tabs.rows[0], vec![Some("1".into()), Some("2".into())]);
    }

    #[test]
    fn reads_json_arrays_and_lines() {
        let source = parse(
            r#"[{"id": 1, "tags": ["a"], "ok": true}, {"id": 2, "name": "x", "ok": null}]"#,
            &ImportFormat::Json,
        )
        .unwrap();
        // Columns come in the order the file first names them.
        assert_eq!(source.columns, ["id", "tags", "ok", "name"]);
        assert_eq!(
            source.rows[0],
            vec![
                Some("1".into()),
                Some("[\"a\"]".into()),
                Some("true".into()),
                None
            ]
        );
        assert_eq!(
            source.rows[1],
            vec![Some("2".into()), None, None, Some("x".into())]
        );
        let lines = parse("{\"a\": 1}\n\n{\"a\": 2}\n", &ImportFormat::JsonLines).unwrap();
        assert_eq!(lines.lines, [1, 3]);
        assert!(parse("{\"a\": 1}", &ImportFormat::Json).is_err());
        assert!(parse("[1]", &ImportFormat::Json).is_err());
    }

    #[test]
    fn converts_by_column_type() {
        let ok = |text: &str, type_name: &str| {
            convert(Some(text), &column("c", type_name, true)).unwrap()
        };
        assert_eq!(ok(" 42 ", "integer"), Value::Int(42));
        assert_eq!(
            ok("0010.50", "numeric(10,2)"),
            Value::Decimal("0010.50".into())
        );
        assert_eq!(ok("1.5", "double precision"), Value::Float(1.5));
        assert_eq!(ok("yes", "boolean"), Value::Bool(true));
        assert_eq!(ok("0", "tinyint(1)"), Value::Bool(false));
        assert_eq!(
            ok("{\"a\":1}", "jsonb"),
            Value::Json(serde_json::json!({"a": 1}))
        );
        assert_eq!(
            ok("2026-01-02", "timestamp with time zone"),
            Value::Temporal("2026-01-02".into())
        );
        assert_eq!(ok(" keep ", "varchar(20)"), Value::Text(" keep ".into()));
        assert_eq!(ok("7", "interval"), Value::Temporal("7".into()));
        // ClickHouse and SQL Server names.
        assert_eq!(ok("42", "Nullable(UInt64)"), Value::Int(42));
        assert_eq!(ok("1.5", "LowCardinality(Float64)"), Value::Float(1.5));
        assert_eq!(
            ok("2026-01-02 10:00:00.5", "DateTime64(3)"),
            Value::Temporal("2026-01-02 10:00:00.5".into())
        );
        assert_eq!(ok("true", "bit"), Value::Bool(true));
        assert_eq!(ok("1", "bit(1)"), Value::Text("1".into()));

        let bad = |text: &str, type_name: &str| {
            convert(Some(text), &column("c", type_name, true)).unwrap_err()
        };
        assert!(bad("12x", "int4").contains("whole number"));
        assert!(bad("1e3", "numeric").contains("not a number"));
        assert!(bad("maybe", "bool").contains("true or false"));
        assert!(bad("{a}", "json").contains("JSON"));

        // Missing values: fine when nullable or defaulted, an error otherwise.
        assert_eq!(convert(None, &column("c", "int", true)), Ok(Value::Null));
        assert!(convert(None, &column("c", "int", false)).is_err());
        let defaulted = ColumnDescription {
            default_value: Some("0".into()),
            ..column("c", "int", false)
        };
        assert_eq!(convert(None, &defaulted), Ok(Value::Null));
    }

    #[test]
    fn maps_rows_and_checks_the_mapping() {
        let table = [
            column("id", "int", false),
            column("email", "text", false),
            column("note", "text", true),
        ];
        let mapping = [
            Mapping {
                source: 1,
                column: "email".into(),
            },
            Mapping {
                source: 0,
                column: "id".into(),
            },
        ];
        let row = [
            Some("7".into()),
            Some("a@x.io".into()),
            Some("ignored".into()),
        ];
        let cells = convert_row(&row, 2, &mapping, &table).unwrap();
        assert_eq!(
            cells[0],
            crate::edit::Cell {
                column: "email".into(),
                value: Value::Text("a@x.io".into())
            }
        );
        assert_eq!(cells[1].value, Value::Int(7));
        let error = convert_row(&[Some("x".into()), None, None], 9, &mapping, &table).unwrap_err();
        assert_eq!((error.line, error.column.as_deref()), (9, Some("email")));

        assert_eq!(check_mapping(&mapping, &table, false).unwrap(), ["note"]);
        assert!(
            check_mapping(&mapping[..1], &table, false)
                .unwrap_err()
                .contains("id is required")
        );
        assert!(check_mapping(&[mapping[0].clone(), mapping[0].clone()], &table, false).is_err());
    }
}
