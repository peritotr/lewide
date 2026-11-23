//! Writing rows to a file: CSV and its relatives, JSON, SQL `INSERT`s and Excel.
//!
//! An [`Exporter`] takes rows a page at a time, so a table of any size goes
//! through without being held in memory; Excel is the exception: the sheet
//! is built in memory and written at the end (see [`crate::xlsx`]).

use std::io::Write;

use base64::Engine;
use serde::Deserialize;

use crate::sql::{self, SqlDialect};
use crate::{Error, Result, Value};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum ExportFormat {
    /// Delimited text as RFC 4180 describes it.
    Csv(CsvOptions),
    /// One array of objects, keyed by column name.
    Json,
    /// One object per line.
    JsonLines,
    /// One `INSERT` statement per row.
    Sql {
        /// The table name as it should appear, already quoted.
        table: String,
        dialect: SqlDialect,
    },
    /// An Excel workbook with one sheet; a header row in bold.
    Xlsx {
        /// The sheet's name; Excel's own (`Sheet1`) when none is given.
        #[serde(default)]
        sheet: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct CsvOptions {
    /// One character: `,`, `;`, a tab or `|`.
    pub delimiter: char,
    /// Column names on the first line.
    pub header: bool,
    /// What a NULL is written as. Never quoted, so with the default (nothing)
    /// a NULL and an empty text stay apart: the empty text is written `""`.
    pub null_text: String,
    /// `\r\n` instead of `\n`.
    pub crlf: bool,
    /// A byte-order mark first, which Excel needs to read UTF-8.
    pub bom: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            header: true,
            null_text: String::new(),
            crlf: false,
            bom: false,
        }
    }
}

pub struct Exporter<W: Write> {
    out: W,
    format: ExportFormat,
    columns: Vec<String>,
    rows: u64,
    sheet: Option<crate::xlsx::SheetWriter>,
}

impl<W: Write> Exporter<W> {
    /// Writes whatever comes before the first row.
    pub fn new(out: W, format: ExportFormat, columns: Vec<String>) -> Result<Self> {
        if let ExportFormat::Csv(options) = &format
            && matches!(options.delimiter, '"' | '\r' | '\n')
        {
            return Err(Error::InvalidConfig(
                "the delimiter cannot be a quote or a line break".into(),
            ));
        }
        let sheet = match &format {
            ExportFormat::Xlsx { sheet } => {
                Some(crate::xlsx::SheetWriter::new(sheet.as_deref(), &columns)?)
            }
            _ => None,
        };
        let mut exporter = Self {
            out,
            format,
            columns,
            rows: 0,
            sheet,
        };
        exporter.start().map_err(file_err)?;
        Ok(exporter)
    }

    fn start(&mut self) -> std::io::Result<()> {
        match &self.format {
            ExportFormat::Csv(options) => {
                if options.bom {
                    self.out.write_all("\u{feff}".as_bytes())?;
                }
                if options.header {
                    let line = self
                        .columns
                        .iter()
                        .map(|name| csv_field(name, options))
                        .collect::<Vec<_>>()
                        .join(&options.delimiter.to_string());
                    self.out.write_all(line.as_bytes())?;
                    self.out.write_all(line_end(options))?;
                }
            }
            ExportFormat::Json => self.out.write_all(b"[")?,
            ExportFormat::JsonLines | ExportFormat::Sql { .. } | ExportFormat::Xlsx { .. } => {}
        }
        Ok(())
    }

    pub fn write_rows(&mut self, rows: &[Vec<Value>]) -> Result<()> {
        for row in rows {
            if row.len() != self.columns.len() {
                return Err(Error::Internal(format!(
                    "a row has {} values for {} columns",
                    row.len(),
                    self.columns.len()
                )));
            }
            if let Some(sheet) = self.sheet.as_mut() {
                sheet.write_row(row)?;
            } else {
                let line = self.row(row)?;
                self.out.write_all(line.as_bytes()).map_err(file_err)?;
            }
            self.rows += 1;
        }
        Ok(())
    }

    fn row(&self, row: &[Value]) -> Result<String> {
        Ok(match &self.format {
            ExportFormat::Csv(options) => {
                let mut line = row
                    .iter()
                    .map(|value| match csv_text(value) {
                        Some(text) => csv_field(&text, options),
                        None => options.null_text.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(&options.delimiter.to_string());
                line.push_str(if options.crlf { "\r\n" } else { "\n" });
                line
            }
            ExportFormat::Json => {
                let separator = if self.rows == 0 { "\n  " } else { ",\n  " };
                format!("{separator}{}", self.json_object(row))
            }
            ExportFormat::JsonLines => format!("{}\n", self.json_object(row)),
            ExportFormat::Sql { table, dialect } => {
                let values = row
                    .iter()
                    .map(|value| sql::literal(*dialect, value))
                    .collect::<Result<Vec<_>>>()?;
                let columns = self
                    .columns
                    .iter()
                    .map(|name| dialect.quote_ident(name))
                    .collect::<Vec<_>>();
                format!(
                    "INSERT INTO {table} ({}) VALUES ({});\n",
                    columns.join(", "),
                    values.join(", ")
                )
            }
            ExportFormat::Xlsx { .. } => unreachable!("Excel rows go to the sheet"),
        })
    }

    fn json_object(&self, row: &[Value]) -> String {
        let fields = self
            .columns
            .iter()
            .zip(row)
            .map(|(name, value)| format!("{}: {}", json_string(name), json_value(value)))
            .collect::<Vec<_>>();
        format!("{{{}}}", fields.join(", "))
    }

    /// Writes whatever follows the last row and returns how many rows went out.
    pub fn finish(mut self) -> Result<u64> {
        if let Some(sheet) = self.sheet.take() {
            let bytes = sheet.finish()?;
            self.out.write_all(&bytes).map_err(file_err)?;
        }
        if self.format == ExportFormat::Json {
            let end: &[u8] = if self.rows == 0 { b"]\n" } else { b"\n]\n" };
            self.out.write_all(end).map_err(file_err)?;
        }
        self.out.flush().map_err(file_err)?;
        Ok(self.rows)
    }
}

fn file_err(error: std::io::Error) -> Error {
    Error::File(error.to_string())
}

fn line_end(options: &CsvOptions) -> &'static [u8] {
    if options.crlf { b"\r\n" } else { b"\n" }
}

/// A value as CSV shows it; `None` for NULL.
fn csv_text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(s) | Value::Text(s) | Value::Temporal(s) => s.clone(),
        Value::Bytes(bytes) => {
            let mut hex = String::with_capacity(2 + bytes.len() * 2);
            hex.push_str("0x");
            for byte in bytes {
                hex.push_str(&format!("{byte:02x}"));
            }
            hex
        }
        Value::Json(json) => json.to_string(),
    })
}

fn csv_field(text: &str, options: &CsvOptions) -> String {
    let needs_quotes = text.is_empty()
        || text == options.null_text
        || text.starts_with(' ')
        || text.ends_with(' ')
        || text
            .chars()
            .any(|c| c == options.delimiter || matches!(c, '"' | '\r' | '\n'));
    if needs_quotes {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

fn json_string(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

fn json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        // JSON has no NaN or infinity.
        Value::Float(f) if f.is_finite() => f.to_string(),
        Value::Float(f) => json_string(&f.to_string()),
        // Written as a number, digit for digit, when JSON can hold it.
        Value::Decimal(text) if is_json_number(text) => text.clone(),
        Value::Decimal(text) | Value::Text(text) | Value::Temporal(text) => json_string(text),
        Value::Bytes(bytes) => {
            json_string(&base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        Value::Json(json) => json.to_string(),
    }
}

/// The JSON number grammar: `-? int frac? exp?`.
fn is_json_number(text: &str) -> bool {
    let rest = text.strip_prefix('-').unwrap_or(text);
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let whole = digits(rest);
    if whole == 0 || (whole > 1 && rest.starts_with('0')) {
        return false;
    }
    let mut rest = &rest[whole..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let count = digits(fraction);
        if count == 0 {
            return false;
        }
        rest = &fraction[count..];
    }
    if let Some(exponent) = rest.strip_prefix(['e', 'E']) {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        let count = digits(exponent);
        if count == 0 {
            return false;
        }
        rest = &exponent[count..];
    }
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(format: ExportFormat, columns: &[&str], rows: &[Vec<Value>]) -> (String, u64) {
        let mut out = Vec::new();
        let mut exporter = Exporter::new(
            &mut out,
            format,
            columns.iter().map(|c| (*c).to_owned()).collect(),
        )
        .unwrap();
        // In two calls, as pages arrive.
        let middle = rows.len() / 2;
        exporter.write_rows(&rows[..middle]).unwrap();
        exporter.write_rows(&rows[middle..]).unwrap();
        let count = exporter.finish().unwrap();
        (String::from_utf8(out).unwrap(), count)
    }

    fn text(s: &str) -> Value {
        Value::Text(s.to_owned())
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        let (out, count) = export(
            ExportFormat::Csv(CsvOptions::default()),
            &["id", "note, with comma"],
            &[
                vec![Value::Int(1), text("plain")],
                vec![Value::Int(2), text("says \"hi\", twice")],
                vec![Value::Int(3), text("two\nlines")],
                vec![Value::Int(4), text(" padded ")],
            ],
        );
        assert_eq!(
            out,
            "id,\"note, with comma\"\n1,plain\n2,\"says \"\"hi\"\", twice\"\n3,\"two\nlines\"\n4,\" padded \"\n"
        );
        assert_eq!(count, 4);
    }

    #[test]
    fn csv_keeps_null_and_empty_text_apart() {
        let rows = [vec![Value::Null, text("")], vec![text("NULL"), text("x")]];
        let (out, _) = export(ExportFormat::Csv(CsvOptions::default()), &["a", "b"], &rows);
        assert_eq!(out, "a,b\n,\"\"\nNULL,x\n");

        // With a word for NULL, the same word as real text is quoted.
        let options = CsvOptions {
            null_text: "NULL".into(),
            ..CsvOptions::default()
        };
        let (out, _) = export(ExportFormat::Csv(options), &["a", "b"], &rows);
        assert_eq!(out, "a,b\nNULL,\"\"\n\"NULL\",x\n");
    }

    #[test]
    fn csv_options_apply() {
        let options = CsvOptions {
            delimiter: ';',
            header: false,
            crlf: true,
            bom: true,
            ..CsvOptions::default()
        };
        let (out, _) = export(
            ExportFormat::Csv(options),
            &["a", "b", "c", "d"],
            &[vec![
                text("1,5"),
                text("x;y"),
                Value::Bytes(vec![0, 255]),
                Value::Json(serde_json::json!({"k": [1]})),
            ]],
        );
        assert_eq!(out, "\u{feff}1,5;\"x;y\";0x00ff;\"{\"\"k\"\":[1]}\"\r\n");

        let bad = CsvOptions {
            delimiter: '"',
            ..CsvOptions::default()
        };
        assert!(Exporter::new(Vec::new(), ExportFormat::Csv(bad), vec![]).is_err());
    }

    #[test]
    fn json_is_valid_and_keeps_numbers_exact() {
        let rows = [
            vec![
                Value::Int(i64::MAX),
                Value::Decimal("12345678901234567890.0100".into()),
                Value::Float(f64::NAN),
                Value::Bytes(vec![0, 255]),
            ],
            vec![
                Value::Null,
                Value::Decimal("NaN".into()),
                Value::Float(1.5),
                Value::Json(serde_json::json!({"a": null})),
            ],
        ];
        let (out, count) = export(ExportFormat::Json, &["i", "d", "f", "quote\"d"], &rows);
        assert_eq!(count, 2);
        assert!(out.contains("9223372036854775807"), "{out}");
        assert!(out.contains("\"d\": 12345678901234567890.0100"), "{out}");
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["f"], "NaN");
        assert_eq!(parsed[0]["quote\"d"], "AP8=");
        assert_eq!(parsed[1]["i"], serde_json::Value::Null);
        assert_eq!(parsed[1]["d"], "NaN");
        assert_eq!(parsed[1]["quote\"d"]["a"], serde_json::Value::Null);

        let (empty, count) = export(ExportFormat::Json, &["a"], &[]);
        assert_eq!((empty.as_str(), count), ("[]\n", 0));

        let (lines, _) = export(ExportFormat::JsonLines, &["i", "d", "f", "b"], &rows);
        assert_eq!(lines.lines().count(), 2);
        for line in lines.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }

    #[test]
    fn recognizes_json_numbers() {
        for ok in ["0", "-0", "10", "1.50", "-12.0e+3", "1E9"] {
            assert!(is_json_number(ok), "{ok}");
        }
        for bad in [
            "", "-", "01", ".5", "1.", "1e", "+1", "NaN", "Infinity", "1 ",
        ] {
            assert!(!is_json_number(bad), "{bad}");
        }
    }

    #[test]
    fn sql_inserts_use_the_dialect() {
        let (out, _) = export(
            ExportFormat::Sql {
                table: "`app`.`people`".into(),
                dialect: SqlDialect::MySql,
            },
            &["id", "na`me"],
            &[vec![Value::Int(1), text("O'Brien")]],
        );
        assert_eq!(
            out,
            "INSERT INTO `app`.`people` (`id`, `na``me`) VALUES (1, 'O''Brien');\n"
        );
    }

    #[test]
    fn a_short_row_is_refused() {
        let mut exporter = Exporter::new(Vec::new(), ExportFormat::Json, vec!["a".into()]).unwrap();
        assert!(exporter.write_rows(&[vec![]]).is_err());
    }
}
