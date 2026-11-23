use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A single cell value, normalized across drivers.
///
/// Serialized as `{ "type": ..., "value": ... }`. 64-bit integers and decimals
/// travel as strings because JavaScript numbers lose precision above 2^53.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Null,
    Bool(bool),
    #[serde(serialize_with = "as_string", deserialize_with = "from_string")]
    Int(i64),
    Float(f64),
    /// Arbitrary-precision number in its canonical text form.
    Decimal(String),
    Text(String),
    #[serde(serialize_with = "as_base64", deserialize_with = "from_base64")]
    Bytes(Vec<u8>),
    Json(serde_json::Value),
    /// ISO 8601 date, time or timestamp as produced by the database.
    Temporal(String),
}

fn as_string<S: Serializer>(v: &i64, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_string())
}

fn as_base64<S: Serializer>(v: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
}

fn from_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<i64, D::Error> {
    String::deserialize(d)?
        .trim()
        .parse()
        .map_err(serde::de::Error::custom)
}

fn from_base64<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<u8>, D::Error> {
    base64::engine::general_purpose::STANDARD
        .decode(String::deserialize(d)?)
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_integers_keep_precision() {
        let json = serde_json::to_string(&Value::Int(i64::MAX)).unwrap();
        assert_eq!(json, r#"{"type":"int","value":"9223372036854775807"}"#);
    }

    #[test]
    fn bytes_are_base64() {
        let json = serde_json::to_string(&Value::Bytes(vec![0, 255])).unwrap();
        assert_eq!(json, r#"{"type":"bytes","value":"AP8="}"#);
    }

    #[test]
    fn values_read_back_as_they_were_written() {
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::Int(i64::MIN),
            Value::Float(1.5),
            Value::Decimal("0.10".into()),
            Value::Text("héllo".into()),
            Value::Bytes(vec![0, 1, 255]),
            Value::Json(serde_json::json!({"a": [1, null]})),
            Value::Temporal("2026-01-02".into()),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&json).unwrap(),
                value,
                "{json}"
            );
        }
        assert!(serde_json::from_str::<Value>(r#"{"type":"int","value":"12x"}"#).is_err());
    }

    #[test]
    fn null_has_no_value() {
        assert_eq!(
            serde_json::to_string(&Value::Null).unwrap(),
            r#"{"type":"null"}"#
        );
    }
}
