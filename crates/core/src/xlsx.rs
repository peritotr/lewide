//! Excel files: one sheet written with `rust_xlsxwriter`, any sheet read with
//! `calamine` (`.xlsx`, `.xlsm`, `.xls`, `.ods`). Both are pure Rust.
//!
//! Writing keeps the cell types a spreadsheet understands: whole numbers and
//! decimals that fit a double as numbers, dates and times as dates, booleans
//! as booleans; anything else as text. Reading turns every cell into text
//! (the import's column mapping converts it), with dates as ISO 8601.

use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, Worksheet};

use crate::import::Source;
use crate::{Error, Result, Value};

/// The most rows one sheet holds, the header included.
pub const MAX_ROWS: u32 = 1_048_576;
/// The longest text a cell holds.
const MAX_TEXT: usize = 32_767;
/// Doubles hold integers exactly up to 2^53; past that a number is written as text.
const EXACT: i64 = 1 << 53;

/// One sheet being written, held in memory until the file is saved.
pub struct SheetWriter {
    sheet: Worksheet,
    row: u32,
    columns: u16,
    date: Format,
    datetime: Format,
}

fn xlsx_err(error: rust_xlsxwriter::XlsxError) -> Error {
    Error::File(format!("Excel: {error}"))
}

impl SheetWriter {
    pub fn new(name: Option<&str>, columns: &[String]) -> Result<Self> {
        let width = u16::try_from(columns.len())
            .ok()
            .filter(|w| *w <= 16_384)
            .ok_or_else(|| Error::File("Excel sheets hold at most 16,384 columns".into()))?;
        let mut sheet = Worksheet::new();
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            sheet.set_name(name).map_err(xlsx_err)?;
        }
        let bold = Format::new().set_bold();
        for (i, column) in columns.iter().enumerate() {
            sheet
                .write_string_with_format(0, i as u16, clip(column), &bold)
                .map_err(xlsx_err)?;
        }
        // The header stays in view while scrolling.
        sheet.set_freeze_panes(1, 0).map_err(xlsx_err)?;
        Ok(Self {
            sheet,
            row: 1,
            columns: width,
            date: Format::new().set_num_format("yyyy-mm-dd"),
            datetime: Format::new().set_num_format("yyyy-mm-dd hh:mm:ss"),
        })
    }

    pub fn write_row(&mut self, row: &[Value]) -> Result<()> {
        if self.row >= MAX_ROWS {
            return Err(Error::File(format!(
                "an Excel sheet holds at most {} rows; export to CSV for more",
                MAX_ROWS - 1
            )));
        }
        for (i, value) in row.iter().enumerate().take(self.columns as usize) {
            self.write_cell(i as u16, value)?;
        }
        self.row += 1;
        Ok(())
    }

    fn write_cell(&mut self, column: u16, value: &Value) -> Result<()> {
        let row = self.row;
        let sheet = &mut self.sheet;
        let text = |sheet: &mut Worksheet, text: &str| {
            sheet.write_string(row, column, clip(text)).map(|_| ())
        };
        let written = match value {
            Value::Null => Ok(()),
            Value::Bool(flag) => sheet.write_boolean(row, column, *flag).map(|_| ()),
            Value::Int(n) if n.abs() <= EXACT => {
                sheet.write_number(row, column, *n as f64).map(|_| ())
            }
            Value::Int(n) => text(sheet, &n.to_string()),
            Value::Float(f) if f.is_finite() => sheet.write_number(row, column, *f).map(|_| ()),
            Value::Float(f) => text(sheet, &f.to_string()),
            Value::Decimal(d) => match exact_double(d) {
                Some(number) => sheet.write_number(row, column, number).map(|_| ()),
                None => text(sheet, d),
            },
            Value::Text(s) => text(sheet, s),
            Value::Temporal(s) => match iso_datetime(s) {
                Some((when, has_time)) => sheet
                    .write_datetime_with_format(
                        row,
                        column,
                        &when,
                        if has_time { &self.datetime } else { &self.date },
                    )
                    .map(|_| ()),
                None => text(sheet, s),
            },
            Value::Bytes(bytes) => {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                text(sheet, &format!("0x{hex}"))
            }
            Value::Json(json) => text(sheet, &json.to_string()),
        };
        written.map_err(xlsx_err)
    }

    /// The finished file.
    pub fn finish(self) -> Result<Vec<u8>> {
        let mut book = Workbook::new();
        book.push_worksheet(self.sheet);
        book.save_to_buffer().map_err(xlsx_err)
    }

    /// Rows written, the header not counted.
    pub fn rows(&self) -> u64 {
        u64::from(self.row - 1)
    }
}

/// A cell holds at most 32,767 characters; longer text is cut there.
fn clip(text: &str) -> &str {
    match text.char_indices().nth(MAX_TEXT) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// A decimal as a double, when the double shows the same digits.
fn exact_double(text: &str) -> Option<f64> {
    let number: f64 = text.parse().ok()?;
    let digits = text.trim_start_matches('-').replace('.', "");
    let significant = digits.trim_start_matches('0').trim_end_matches('0').len();
    (number.is_finite() && significant <= 15).then_some(number)
}

/// `2023-11-14`, `2023-11-14T22:13:20[.fff]` or with a space; a zone is not
/// kept by Excel, so a value with one stays text.
fn iso_datetime(text: &str) -> Option<(ExcelDateTime, bool)> {
    let text = text.trim();
    let bytes = text.as_bytes();
    let date_shape = bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[7] == b'-';
    if !date_shape {
        return None;
    }
    let date = ExcelDateTime::from_ymd(
        text[..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..10].parse().ok()?,
    )
    .ok()?;
    if bytes.len() == 10 {
        return Some((date, false));
    }
    let time = &text[11..];
    if !matches!(bytes[10], b'T' | b' ')
        || time.len() < 8
        || time.contains(['+', 'Z'])
        || time[8..].contains('-')
    {
        return None;
    }
    let seconds: f64 = time[6..].parse().ok()?;
    let when = date
        .and_hms(time[..2].parse().ok()?, time[3..5].parse().ok()?, seconds)
        .ok()?;
    Some((when, true))
}

/// The sheets of a workbook, in order.
pub fn sheet_names(path: &std::path::Path) -> Result<Vec<String>> {
    use calamine::Reader;
    let book = calamine::open_workbook_auto(path).map_err(read_err)?;
    Ok(book.sheet_names())
}

fn read_err(error: calamine::Error) -> Error {
    Error::File(format!("Excel: {error}"))
}

/// A sheet (the first when `sheet` is `None`) as rows of text.
pub fn read(path: &std::path::Path, sheet: Option<&str>, header: bool) -> Result<Source> {
    use calamine::Reader;
    let mut book = calamine::open_workbook_auto(path).map_err(read_err)?;
    let name = match sheet.filter(|s| !s.is_empty()) {
        Some(name) => name.to_owned(),
        None => book
            .sheet_names()
            .first()
            .cloned()
            .ok_or_else(|| Error::File("the workbook has no sheets".into()))?,
    };
    let range = book.worksheet_range(&name).map_err(read_err)?;
    // Row numbers as the spreadsheet shows them, 1-based.
    let first = range.start().map_or(0, |(row, _)| row) + 1;
    let mut rows = range
        .rows()
        .map(|cells| cells.iter().map(cell_text).collect::<Vec<_>>());
    let width = range.get_size().1;
    let columns = if header {
        rows.next()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                name.filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| format!("column_{}", i + 1))
            })
            .collect()
    } else {
        (1..=width).map(|i| format!("column_{i}")).collect()
    };
    let skip = u32::from(header);
    let mut source = Source {
        columns,
        rows: Vec::new(),
        lines: Vec::new(),
    };
    for (i, row) in rows.enumerate() {
        // Rows left wholly empty in the sheet are not rows of data.
        if row.iter().all(Option::is_none) {
            continue;
        }
        source.lines.push(first + skip + i as u32);
        source.rows.push(row);
    }
    Ok(source)
}

/// A cell as the import reads it: text, with dates and times as ISO 8601.
fn cell_text(cell: &calamine::Data) -> Option<String> {
    use calamine::Data;
    Some(match cell {
        Data::Empty => return None,
        Data::String(s) => s.clone(),
        Data::Int(n) => n.to_string(),
        // Whole numbers without ".0", as they were typed.
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(when) if when.is_datetime() => serial_to_iso(when.as_f64()),
        Data::DateTime(when) => when.as_f64().to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(error) => format!("#{error}"),
    })
}

/// An Excel serial date (days since 1899-12-30, the 1900 system) as ISO 8601:
/// a date alone when there is no time of day.
fn serial_to_iso(serial: f64) -> String {
    let ms = (serial * 86_400_000.0).round() as i64;
    let days = ms.div_euclid(86_400_000) - 25_569;
    let rest = ms.rem_euclid(86_400_000);
    // Howard Hinnant's days-to-civil conversion, from 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if rest == 0 {
        return format!("{year:04}-{month:02}-{day:02}");
    }
    let mut text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60
    );
    if rest % 1000 != 0 {
        text.push_str(&format!(".{:03}", rest % 1000));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back_a_sheet() {
        let dir = std::env::temp_dir().join(format!("dbclient-xlsx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.xlsx");
        let columns = vec![
            "id".to_owned(),
            "name".to_owned(),
            "price".to_owned(),
            "when".to_owned(),
            "ok".to_owned(),
            "big".to_owned(),
        ];
        let mut sheet = SheetWriter::new(Some("Satışlar"), &columns).unwrap();
        sheet
            .write_row(&[
                Value::Int(1),
                Value::Text("Çağrı".into()),
                Value::Decimal("12.50".into()),
                Value::Temporal("2023-11-14T22:13:20".into()),
                Value::Bool(true),
                Value::Int(9_007_199_254_740_993),
            ])
            .unwrap();
        sheet
            .write_row(&[
                Value::Int(2),
                Value::Null,
                Value::Decimal("0.1234567890123456789".into()),
                Value::Temporal("2024-02-29".into()),
                Value::Bool(false),
                Value::Json(serde_json::json!({"a": 1})),
            ])
            .unwrap();
        assert_eq!(sheet.rows(), 2);
        std::fs::write(&path, sheet.finish().unwrap()).unwrap();

        assert_eq!(sheet_names(&path).unwrap(), ["Satışlar"]);
        let source = read(&path, None, true).unwrap();
        assert_eq!(source.columns, columns);
        assert_eq!(source.lines, [2, 3]);
        let text = |s: &str| Some(s.to_owned());
        assert_eq!(
            source.rows[0],
            [
                text("1"),
                text("Çağrı"),
                text("12.5"),
                text("2023-11-14T22:13:20"),
                text("true"),
                text("9007199254740993")
            ]
        );
        // Digits a double would lose stay text; a date alone reads as one.
        assert_eq!(
            source.rows[1],
            [
                text("2"),
                None,
                text("0.1234567890123456789"),
                text("2024-02-29"),
                text("false"),
                text("{\"a\":1}")
            ]
        );

        let headless = read(&path, Some("Satışlar"), false).unwrap();
        assert_eq!(headless.columns[0], "column_1");
        assert_eq!(headless.rows.len(), 3);
        assert!(read(&path, Some("nope"), true).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dates_and_numbers() {
        assert_eq!(serial_to_iso(45244.0), "2023-11-14");
        assert_eq!(
            serial_to_iso(45_244.0 + 80_000.0 / 86_400.0),
            "2023-11-14T22:13:20"
        );
        assert!(iso_datetime("2023-11-14T22:13:20+00").is_none());
        assert!(iso_datetime("14.11.2023").is_none());
        assert!(iso_datetime("2023-11-14 08:00:00.5").is_some_and(|(_, time)| time));
        assert_eq!(exact_double("12.50"), Some(12.5));
        assert_eq!(exact_double("12345678901234567"), None);
        assert_eq!(clip(&"é".repeat(40_000)).chars().count(), MAX_TEXT);
    }
}
