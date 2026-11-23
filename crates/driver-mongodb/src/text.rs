//! Reads the query documents a user types: filters, projections, sort orders,
//! and documents to store.
//!
//! The syntax is Extended JSON (canonical or relaxed) plus the habits people
//! bring from the MongoDB shell: unquoted keys, single-quoted strings, a
//! trailing comma, and the constructors `ObjectId("…")`, `ISODate("…")`,
//! `NumberInt(…)`, `NumberLong(…)`, `NumberDecimal("…")` and `UUID("…")`.
//!
//! It is written by hand instead of using a JSON library so that every error
//! names the character it was found at, in words a user can act on.

use mongodb::bson::{self, Bson, Document};

/// Why a text could not be read. `position` is a 1-based character offset.
#[derive(Debug, PartialEq)]
pub struct TextError {
    pub message: String,
    pub position: Option<u32>,
}

/// Keys that make an object a typed value in Extended JSON instead of a
/// sub-document.
const WRAPPERS: [&str; 16] = [
    "$oid",
    "$symbol",
    "$regularExpression",
    "$numberInt",
    "$numberLong",
    "$numberDouble",
    "$numberDecimal",
    "$binary",
    "$uuid",
    "$code",
    "$timestamp",
    "$date",
    "$minKey",
    "$maxKey",
    "$dbPointer",
    "$undefined",
];

/// Parses one document. `None` when the text is blank.
pub fn parse_document(text: &str) -> Result<Option<Document>, TextError> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    parser.skip_space();
    if parser.peek().is_none() {
        return Ok(None);
    }
    if parser.peek() != Some('{') {
        return Err(parser.error("a document is expected: { … }"));
    }
    let start = parser.at;
    let (value, _) = parser.object()?;
    parser.skip_space();
    if parser.peek().is_some() {
        return Err(parser.error("nothing is expected after the closing }"));
    }
    match value {
        Bson::Document(document) => Ok(Some(document)),
        // `{"$oid": "…"}` on its own is a value, not a document.
        _ => Err(parser.error_at(start, "a document is expected, not a single value")),
    }
}

/// Parses an aggregation pipeline: a list of stage documents. `None` when the text is blank.
pub fn parse_pipeline(text: &str) -> Result<Option<Vec<Document>>, TextError> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    parser.skip_space();
    if parser.peek().is_none() {
        return Ok(None);
    }
    // A single stage may be written without the brackets.
    let single = parser.peek() == Some('{');
    if !single && parser.peek() != Some('[') {
        return Err(parser.error("a pipeline is expected: [ { $match: … }, … ]"));
    }
    let start = parser.at;
    let value = if single {
        parser.object()?.0
    } else {
        parser.array()?
    };
    parser.skip_space();
    if parser.peek().is_some() {
        return Err(parser.error(if single {
            "nothing is expected after the closing }"
        } else {
            "nothing is expected after the closing ]"
        }));
    }
    let stages = match value {
        Bson::Document(stage) => vec![Bson::Document(stage)],
        Bson::Array(stages) => stages,
        _ => return Err(parser.error_at(start, "a pipeline is expected")),
    };
    stages
        .into_iter()
        .map(|stage| match stage {
            Bson::Document(stage) => Ok(stage),
            _ => Err(parser.error_at(start, "every stage is a document: { $stage: … }")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Turns one value in Extended JSON (as the interface holds an `_id`) into BSON.
pub fn value_from_json(value: &serde_json::Value) -> Result<Bson, String> {
    Bson::try_from(value.clone()).map_err(|e| e.to_string())
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn error(&self, message: impl Into<String>) -> TextError {
        self.error_at(self.at, message)
    }

    fn error_at(&self, index: usize, message: impl Into<String>) -> TextError {
        TextError {
            message: message.into(),
            position: Some(index as u32 + 1),
        }
    }

    /// `after` is the key the value belongs to, for the error message.
    fn value(&mut self, after: Option<&str>) -> Result<Bson, TextError> {
        self.value_and_json(after).map(|(value, _)| value)
    }

    /// A value, and the same value as plain JSON when it was written as such.
    ///
    /// The JSON is needed one level up: an Extended JSON wrapper such as
    /// `{"$timestamp": {"t": 1, "i": 2}}` is defined over the JSON text, and
    /// can no longer be recognised once its inside has become typed values.
    fn value_and_json(
        &mut self,
        after: Option<&str>,
    ) -> Result<(Bson, Option<serde_json::Value>), TextError> {
        self.skip_space();
        if self.peek() == Some('{') {
            return self.object();
        }
        let value = self.plain_value(after)?;
        let json = match &value {
            Bson::String(s) => Some(serde_json::Value::from(s.as_str())),
            Bson::Int32(i) => Some(serde_json::Value::from(*i)),
            Bson::Int64(i) => Some(serde_json::Value::from(*i)),
            Bson::Double(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number),
            Bson::Boolean(b) => Some(serde_json::Value::from(*b)),
            Bson::Null => Some(serde_json::Value::Null),
            // Lists and shell constructors never occur inside a wrapper.
            _ => None,
        };
        Ok((value, json))
    }

    fn plain_value(&mut self, after: Option<&str>) -> Result<Bson, TextError> {
        match self.peek() {
            Some('[') => self.array(),
            Some('"' | '\'') => self.string().map(Bson::String),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) if is_word_start(c) => self.word(),
            _ => Err(self.error(match after {
                Some(key) => format!("a value is expected after {key}:"),
                None => "a value is expected".into(),
            })),
        }
    }

    fn object(&mut self) -> Result<(Bson, Option<serde_json::Value>), TextError> {
        let start = self.at;
        self.at += 1; // {
        let mut document = Document::new();
        // The same fields as JSON, for as long as every one of them has that form.
        let mut json = Some(serde_json::Map::new());
        loop {
            self.skip_space();
            let key = match self.peek() {
                Some('}') => {
                    self.at += 1;
                    break;
                }
                Some('"' | '\'') => self.string()?,
                Some(c) if is_word_start(c) => self.bare_word(),
                None => return Err(self.error("the document is not closed: } is missing")),
                Some(_) => return Err(self.error("a field name is expected")),
            };
            self.skip_space();
            if self.peek() != Some(':') {
                return Err(self.error(format!(": is expected after {key}")));
            }
            self.at += 1;
            let (value, value_json) = self.value_and_json(Some(&key))?;
            match (json.as_mut(), value_json) {
                (Some(map), Some(value_json)) => {
                    map.insert(key.clone(), value_json);
                }
                _ => json = None,
            }
            document.insert(key, value);

            self.skip_space();
            match self.peek() {
                Some(',') => self.at += 1,
                Some('}') => {}
                None => return Err(self.error("the document is not closed: } is missing")),
                Some(_) => return Err(self.error("a , or } is expected")),
            }
        }

        let json = json.map(serde_json::Value::Object);
        let Some(wrapper) = WRAPPERS.iter().find(|w| document.contains_key(**w)) else {
            return Ok((Bson::Document(document), json));
        };
        // The bson crate knows the exact shape of every wrapper; it reads them from JSON.
        let value = json
            .clone()
            .and_then(|json| Bson::try_from(json).ok())
            .ok_or_else(|| self.error_at(start, format!("the value of {wrapper} is not valid")))?;
        // The JSON is passed on as written: wrappers nest, as in
        // `{"$date": {"$numberLong": "0"}}`.
        Ok((value, json))
    }

    fn array(&mut self) -> Result<Bson, TextError> {
        self.at += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                Some(']') => {
                    self.at += 1;
                    return Ok(Bson::Array(items));
                }
                None => return Err(self.error("the list is not closed: ] is missing")),
                _ => items.push(self.value(None)?),
            }
            self.skip_space();
            match self.peek() {
                Some(',') => self.at += 1,
                Some(']') => {}
                None => return Err(self.error("the list is not closed: ] is missing")),
                Some(_) => return Err(self.error("a , or ] is expected")),
            }
        }
    }

    fn string(&mut self) -> Result<String, TextError> {
        let start = self.at;
        let quote = self.chars[self.at];
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error_at(start, "the text is not closed: a quote is missing"));
            };
            self.at += 1;
            match c {
                c if c == quote => return Ok(out),
                '\\' => {
                    let escape_at = self.at - 1;
                    let Some(e) = self.peek() else {
                        return Err(
                            self.error_at(start, "the text is not closed: a quote is missing")
                        );
                    };
                    self.at += 1;
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        '"' | '\'' | '\\' | '/' => e,
                        'u' => self.unicode_escape(escape_at)?,
                        _ => {
                            return Err(
                                self.error_at(escape_at, format!("\\{e} is not a known escape"))
                            );
                        }
                    });
                }
                c => out.push(c),
            }
        }
    }

    /// The part after `\u`, including the second half of a surrogate pair.
    fn unicode_escape(&mut self, escape_at: usize) -> Result<char, TextError> {
        let bad = |p: &Self| p.error_at(escape_at, "\\u needs four hexadecimal digits");
        let unit = |p: &mut Self| -> Result<u32, TextError> {
            let digits: String = p.chars.iter().skip(p.at).take(4).collect();
            if digits.chars().count() != 4 {
                return Err(bad(p));
            }
            p.at += 4;
            u32::from_str_radix(&digits, 16).map_err(|_| bad(p))
        };
        let first = unit(self)?;
        let code = if (0xD800..0xDC00).contains(&first) {
            if self.peek() != Some('\\') || self.chars.get(self.at + 1) != Some(&'u') {
                return Err(bad(self));
            }
            self.at += 2;
            let second = unit(self)?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(bad(self));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| bad(self))
    }

    fn number(&mut self) -> Result<Bson, TextError> {
        let start = self.at;
        if self.peek() == Some('-') {
            self.at += 1;
            if self.chars[self.at..].starts_with(&['I', 'n', 'f', 'i', 'n', 'i', 't', 'y']) {
                self.at += 8;
                return Ok(Bson::Double(f64::NEG_INFINITY));
            }
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
        {
            // A sign belongs to the number only right after the exponent mark.
            if matches!(self.peek(), Some('+' | '-'))
                && !matches!(self.chars[self.at - 1], 'e' | 'E')
            {
                break;
            }
            self.at += 1;
        }
        let text: String = self.chars[start..self.at].iter().collect();
        // The same reading as relaxed Extended JSON: the smallest integer type
        // that holds the number, and a double for everything else.
        if let Ok(i) = text.parse::<i64>() {
            return Ok(match i32::try_from(i) {
                Ok(small) => Bson::Int32(small),
                Err(_) => Bson::Int64(i),
            });
        }
        let digits_only = text.trim_start_matches('-');
        let looks_numeric = digits_only.starts_with(|c: char| c.is_ascii_digit())
            && digits_only
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
        match text.parse::<f64>() {
            Ok(f) if looks_numeric => Ok(Bson::Double(f)),
            _ => Err(self.error_at(start, format!("{text} is not a number"))),
        }
    }

    fn bare_word(&mut self) -> String {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|c| is_word_start(c) || c.is_ascii_digit() || c == '.')
        {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    /// `true`, `null`, … or a shell constructor such as `ObjectId("…")`.
    fn word(&mut self) -> Result<Bson, TextError> {
        let start = self.at;
        let mut word = self.bare_word();
        if word == "new" {
            self.skip_space();
            word = self.bare_word();
        }
        match word.as_str() {
            "true" => return Ok(Bson::Boolean(true)),
            "false" => return Ok(Bson::Boolean(false)),
            "null" => return Ok(Bson::Null),
            "undefined" => return Ok(Bson::Undefined),
            "NaN" => return Ok(Bson::Double(f64::NAN)),
            "Infinity" => return Ok(Bson::Double(f64::INFINITY)),
            _ => {}
        }
        self.skip_space();
        if self.peek() != Some('(') {
            return Err(self.error_at(
                start,
                format!("{word} is not a value; text needs quotes: \"{word}\""),
            ));
        }
        self.at += 1;
        self.skip_space();
        let argument = if self.peek() == Some(')') {
            None
        } else {
            Some(self.value(None)?)
        };
        self.skip_space();
        if self.peek() != Some(')') {
            return Err(self.error(format!("a ) is expected to close {word}(")));
        }
        self.at += 1;

        let invalid = |what: &str| self.error_at(start, format!("{word}(…) needs {what}"));
        // Numbers may be given bare or as text: NumberLong(5) and NumberLong("5").
        let as_text = |argument: &Option<Bson>| match argument {
            Some(Bson::String(s)) => Some(s.clone()),
            Some(Bson::Int32(i)) => Some(i.to_string()),
            Some(Bson::Int64(i)) => Some(i.to_string()),
            Some(Bson::Double(f)) => Some(f.to_string()),
            _ => None,
        };
        match word.as_str() {
            "ObjectId" => match &argument {
                None => Ok(Bson::ObjectId(bson::oid::ObjectId::new())),
                Some(Bson::String(hex)) => bson::oid::ObjectId::parse_str(hex)
                    .map(Bson::ObjectId)
                    .map_err(|_| invalid("24 hexadecimal digits in quotes")),
                _ => Err(invalid("24 hexadecimal digits in quotes")),
            },
            "ISODate" | "Date" => match &argument {
                None => Ok(Bson::DateTime(bson::DateTime::now())),
                Some(Bson::String(text)) => parse_date(text)
                    .map(Bson::DateTime)
                    .ok_or_else(|| invalid("a date such as \"2026-01-31T12:00:00Z\"")),
                // Milliseconds since 1970, as JavaScript's Date takes them.
                Some(Bson::Int32(ms)) => {
                    Ok(Bson::DateTime(bson::DateTime::from_millis(*ms as i64)))
                }
                Some(Bson::Int64(ms)) => Ok(Bson::DateTime(bson::DateTime::from_millis(*ms))),
                _ => Err(invalid("a date such as \"2026-01-31T12:00:00Z\"")),
            },
            "NumberInt" => as_text(&argument)
                .and_then(|t| t.parse().ok())
                .map(Bson::Int32)
                .ok_or_else(|| invalid("a whole number that fits 32 bits")),
            "NumberLong" => as_text(&argument)
                .and_then(|t| t.parse().ok())
                .map(Bson::Int64)
                .ok_or_else(|| invalid("a whole number that fits 64 bits")),
            "NumberDecimal" => as_text(&argument)
                .and_then(|t| t.parse().ok())
                .map(Bson::Decimal128)
                .ok_or_else(|| invalid("a decimal number")),
            "UUID" => match &argument {
                Some(Bson::String(text)) => bson::Uuid::parse_str(text)
                    .map(|u| Bson::Binary(u.into()))
                    .map_err(|_| invalid("a UUID in quotes")),
                _ => Err(invalid("a UUID in quotes")),
            },
            _ => Err(self.error_at(start, format!("{word}(…) is not known"))),
        }
    }
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

/// Accepts what the shell's `ISODate` does in practice: a full timestamp, one
/// without a time zone (read as UTC), or a date alone (midnight UTC).
fn parse_date(text: &str) -> Option<bson::DateTime> {
    let text = text.trim();
    [
        text.to_owned(),
        format!("{text}Z"),
        format!("{text}T00:00:00Z"),
    ]
    .iter()
    .find_map(|candidate| bson::DateTime::parse_rfc3339_str(candidate).ok())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;

    fn parse(text: &str) -> Document {
        parse_document(text).unwrap().unwrap()
    }

    fn error(text: &str) -> (String, u32) {
        let e = parse_document(text).unwrap_err();
        (e.message, e.position.unwrap())
    }

    #[test]
    fn blank_text_is_no_document() {
        assert_eq!(parse_document("  \n ").unwrap(), None);
    }

    #[test]
    fn reads_strict_json() {
        assert_eq!(
            parse(r#"{"a": 1, "b": [true, null, "x\né😀"], "c": {"d": -2.5e1}}"#),
            doc! {"a": 1, "b": [true, Bson::Null, "x\né😀"], "c": {"d": -25.0}}
        );
    }

    #[test]
    fn reads_shell_style() {
        assert_eq!(
            parse("{ price: { $lt: 10 }, 'name': 'a\"b', address.city: \"X\", tags: [1, 2,], }"),
            doc! {"price": {"$lt": 10}, "name": "a\"b", "address.city": "X", "tags": [1, 2]}
        );
    }

    #[test]
    fn numbers_take_the_smallest_type_that_holds_them() {
        let d =
            parse("{a: 2147483647, b: 2147483648, c: 1.0, d: 1e3, e: -0, f: 99999999999999999999}");
        assert_eq!(d.get("a"), Some(&Bson::Int32(i32::MAX)));
        assert_eq!(d.get("b"), Some(&Bson::Int64(2147483648)));
        assert_eq!(d.get("c"), Some(&Bson::Double(1.0)));
        assert_eq!(d.get("d"), Some(&Bson::Double(1000.0)));
        assert_eq!(d.get("e"), Some(&Bson::Int32(0)));
        assert_eq!(d.get("f"), Some(&Bson::Double(1e20)));
    }

    #[test]
    fn reads_extended_json_wrappers() {
        let d = parse(
            r#"{"_id": {"$oid": "65f1c0a2b7e4d3a1c0ffee01"}, "n": {"$numberLong": "5"},
                "at": {"$date": {"$numberLong": "1700000000000"}}, "at2": {"$date": "2023-11-14T22:13:20Z"},
                "f": {"$numberDouble": "1.5"}, "i": {"$numberInt": "7"},
                "ts": {"$timestamp": {"t": 1700000000, "i": 1}},
                "js": {"$code": "f()", "$scope": {"x": 1}}}"#,
        );
        assert_eq!(
            d.get("ts"),
            Some(&Bson::Timestamp(bson::Timestamp {
                time: 1700000000,
                increment: 1
            }))
        );
        assert_eq!(
            d.get("js").unwrap().element_type(),
            bson::spec::ElementType::JavaScriptCodeWithScope
        );
        assert_eq!(
            d.get("_id"),
            Some(&Bson::ObjectId(
                bson::oid::ObjectId::parse_str("65f1c0a2b7e4d3a1c0ffee01").unwrap()
            ))
        );
        assert_eq!(d.get("n"), Some(&Bson::Int64(5)));
        assert_eq!(
            d.get("at"),
            Some(&Bson::DateTime(bson::DateTime::from_millis(1700000000000)))
        );
        assert_eq!(d.get("at"), d.get("at2"));
        assert_eq!(d.get("f"), Some(&Bson::Double(1.5)));
        assert_eq!(d.get("i"), Some(&Bson::Int32(7)));
    }

    #[test]
    fn reads_shell_constructors() {
        let d = parse(
            "{_id: ObjectId('65f1c0a2b7e4d3a1c0ffee01'), at: ISODate(\"2023-11-14\"), \
              n: NumberLong(5), m: NumberLong(\"6\"), i: NumberInt(7), \
              p: NumberDecimal(\"12.50\"), at2: new Date(1700000000000)}",
        );
        assert_eq!(
            d.get("_id").unwrap().element_type(),
            bson::spec::ElementType::ObjectId
        );
        assert_eq!(
            d.get("at"),
            Some(&Bson::DateTime(bson::DateTime::from_millis(1699920000000)))
        );
        assert_eq!(d.get("n"), Some(&Bson::Int64(5)));
        assert_eq!(d.get("m"), Some(&Bson::Int64(6)));
        assert_eq!(d.get("i"), Some(&Bson::Int32(7)));
        assert_eq!(d.get("p").unwrap().to_string(), "12.50");
        assert_eq!(
            d.get("at2"),
            Some(&Bson::DateTime(bson::DateTime::from_millis(1700000000000)))
        );
    }

    #[test]
    fn query_operators_are_not_mistaken_for_wrappers() {
        assert_eq!(
            parse(r#"{"name": {"$regex": "^a", "$options": "i"}, "t": {"$type": "date"}}"#),
            doc! {"name": {"$regex": "^a", "$options": "i"}, "t": {"$type": "date"}}
        );
    }

    #[test]
    fn reads_a_pipeline_with_or_without_brackets() {
        let stages = parse_pipeline(
            "[ { $match: { qty: { $gt: 1 } } }, { $group: { _id: '$city', n: { $sum: 1 } } }, ]",
        )
        .unwrap()
        .unwrap();
        assert_eq!(stages.len(), 2);
        assert_eq!(
            stages[1]
                .get_document("$group")
                .unwrap()
                .get_str("_id")
                .unwrap(),
            "$city"
        );
        assert_eq!(parse_pipeline("{ $limit: 3 }").unwrap().unwrap().len(), 1);
        assert_eq!(parse_pipeline("  ").unwrap(), None);
        assert_eq!(
            parse_pipeline("[1]").unwrap_err().message,
            "every stage is a document: { $stage: … }"
        );
        assert_eq!(parse_pipeline("[] x").unwrap_err().position, Some(4));
        assert!(parse_pipeline("$match").is_err());
    }

    #[test]
    fn errors_say_what_and_where() {
        assert_eq!(
            error("{ price: { $lt: } }"),
            ("a value is expected after $lt:".into(), 17)
        );
        assert_eq!(
            error("{ a: 1"),
            ("the document is not closed: } is missing".into(), 7)
        );
        assert_eq!(error("{ a 1 }"), (": is expected after a".into(), 5));
        assert_eq!(error("{ a: 1 b: 2 }"), ("a , or } is expected".into(), 8));
        assert_eq!(error("[1]"), ("a document is expected: { … }".into(), 1));
        assert_eq!(
            error("{} x"),
            ("nothing is expected after the closing }".into(), 4)
        );
        assert_eq!(
            error("{ a: 'open }"),
            ("the text is not closed: a quote is missing".into(), 6)
        );
        assert_eq!(
            error("{ a: abc }"),
            ("abc is not a value; text needs quotes: \"abc\"".into(), 6)
        );
        assert_eq!(error("{ a: 1.2.3 }"), ("1.2.3 is not a number".into(), 6));
        assert_eq!(error("{ a: [1, 2 }"), ("a , or ] is expected".into(), 12));
        assert_eq!(
            error(r#"{ _id: {"$oid": "xyz"} }"#),
            ("the value of $oid is not valid".into(), 8)
        );
        assert_eq!(
            error("{ _id: ObjectId(\"xyz\") }"),
            (
                "ObjectId(…) needs 24 hexadecimal digits in quotes".into(),
                8
            )
        );
        assert_eq!(
            error(r#"{"$oid": "65f1c0a2b7e4d3a1c0ffee01"}"#),
            ("a document is expected, not a single value".into(), 1)
        );
    }

    #[test]
    fn positions_count_characters_not_bytes() {
        assert_eq!(error("{ é: 'ü', b: }").1, 14);
    }
}
