//! SQL text for the vector operations. Pure functions, so they can be tested
//! without a server. Identifiers arrive already quoted; everything a user typed
//! goes through [`literal`].

use dbclient_core::vector::{Condition, ConditionOp};

/// How distances are measured. pgvector keeps no metric on the column: the
/// operator class of the column's index says which one the data was meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cosine,
    Euclid,
    Dot,
    Manhattan,
}

impl Metric {
    /// From an operator class name (`vector_cosine_ops`) or what a person
    /// writes in the driver options (`cosine`, `l2`, `ip`, `l1`).
    pub fn parse(text: &str) -> Option<Metric> {
        let text = text.trim().to_ascii_lowercase();
        let text = text.strip_prefix("vector_").unwrap_or(&text);
        let text = text.strip_suffix("_ops").unwrap_or(text);
        match text {
            "cosine" => Some(Metric::Cosine),
            "l2" | "euclid" | "euclidean" => Some(Metric::Euclid),
            "ip" | "dot" | "inner_product" => Some(Metric::Dot),
            "l1" | "manhattan" | "taxicab" => Some(Metric::Manhattan),
            _ => None,
        }
    }

    /// The name the vector view shows, the same words Qdrant uses.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Cosine => "Cosine",
            Metric::Euclid => "Euclid",
            Metric::Dot => "Dot",
            Metric::Manhattan => "Manhattan",
        }
    }

    pub fn higher_is_better(self) -> bool {
        matches!(self, Metric::Cosine | Metric::Dot)
    }

    /// The distance operator. Ordering by it ascending is what an index serves.
    pub fn operator(self) -> &'static str {
        match self {
            Metric::Cosine => "<=>",
            Metric::Euclid => "<->",
            Metric::Dot => "<#>",
            Metric::Manhattan => "<+>",
        }
    }

    /// The score shown for a distance: cosine similarity and the inner product
    /// are turned back into "higher is closer"; the other two stay distances.
    pub fn score(self, distance: &str) -> String {
        match self {
            Metric::Cosine => format!("1 - ({distance})"),
            Metric::Dot => format!("({distance}) * -1"),
            Metric::Euclid | Metric::Manhattan => distance.to_owned(),
        }
    }
}

/// A string literal. `standard_conforming_strings` is on by default since
/// PostgreSQL 9.1, so doubling the quotes is all the escaping needed.
pub fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A vector constant: `'[0.1,0.2]'::vector`.
pub fn vector_literal(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("'[{}]'::vector", parts.join(","))
}

/// Reads pgvector's text form, `[0.1,0.2,0.3]`.
pub fn parse_vector(text: &str) -> Option<Vec<f32>> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner.split(',').map(|v| v.trim().parse().ok()).collect()
}

/// The JSON value of a payload field. Nested fields are written with dots, as
/// in the other vector stores: `address.city` reads `{"address": {"city": …}}`.
fn field(row: &str, name: &str) -> String {
    let path: Vec<String> = name
        .split('.')
        .map(|part| format!("\"{}\"", part.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!(
        "(to_jsonb({row}) #> {})",
        literal(&format!("{{{}}}", path.join(",")))
    )
}

fn json(value: &serde_json::Value) -> String {
    format!("{}::jsonb", literal(&value.to_string()))
}

/// One payload condition as SQL over the row `row`. Follows the vector view's
/// rules: a list matches when any item does, and `Ne` also matches rows
/// without the field.
pub fn condition(row: &str, c: &Condition) -> Result<String, String> {
    let j = field(row, &c.field);
    let compare = |op: &str| -> Result<String, String> {
        match &c.value {
            serde_json::Value::Number(n) => Ok(format!(
                "(CASE WHEN jsonb_typeof({j}) = 'number' THEN ({j} #>> '{{}}')::numeric {op} {n} END)"
            )),
            // Dates and times are compared as their ISO text.
            serde_json::Value::String(s) => Ok(format!(
                "(CASE WHEN jsonb_typeof({j}) = 'string' THEN ({j} #>> '{{}}') {op} {} END)",
                literal(s)
            )),
            _ => Err(format!("{} needs a number or a date", c.field)),
        }
    };
    Ok(match c.op {
        ConditionOp::Eq => {
            let v = json(&c.value);
            format!(
                "({j} = {v} OR (jsonb_typeof({j}) = 'array' AND {j} @> jsonb_build_array({v})))"
            )
        }
        ConditionOp::Ne => format!("({j} IS DISTINCT FROM {})", json(&c.value)),
        ConditionOp::Gt => compare(">")?,
        ConditionOp::Gte => compare(">=")?,
        ConditionOp::Lt => compare("<")?,
        ConditionOp::Lte => compare("<=")?,
        ConditionOp::In => {
            if !c.value.is_array() {
                return Err(format!("{} needs a list of values", c.field));
            }
            format!("({} @> jsonb_build_array({j}))", json(&c.value))
        }
        ConditionOp::IsNull => format!("({j} IS NULL OR {j} IN ('null'::jsonb, '[]'::jsonb))"),
        ConditionOp::IsNotNull => {
            format!("NOT ({j} IS NULL OR {j} IN ('null'::jsonb, '[]'::jsonb))")
        }
    })
}

/// All conditions joined with AND; `TRUE` when there are none.
pub fn filter(row: &str, conditions: &[Condition]) -> Result<String, String> {
    if conditions.is_empty() {
        return Ok("TRUE".into());
    }
    let parts = conditions
        .iter()
        .map(|c| condition(row, c))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cond(field: &str, op: ConditionOp, value: serde_json::Value) -> Condition {
        Condition {
            field: field.into(),
            op,
            value,
        }
    }

    #[test]
    fn metrics_are_read_from_operator_classes_and_options() {
        assert_eq!(Metric::parse("vector_cosine_ops"), Some(Metric::Cosine));
        assert_eq!(Metric::parse("vector_l2_ops"), Some(Metric::Euclid));
        assert_eq!(Metric::parse("vector_ip_ops"), Some(Metric::Dot));
        assert_eq!(Metric::parse("vector_l1_ops"), Some(Metric::Manhattan));
        assert_eq!(Metric::parse(" L2 "), Some(Metric::Euclid));
        assert_eq!(Metric::parse("hamming"), None);
    }

    #[test]
    fn literals_cannot_end_the_string() {
        assert_eq!(literal("it's"), "'it''s'");
        assert_eq!(literal("x'; DROP TABLE t; --"), "'x''; DROP TABLE t; --'");
    }

    #[test]
    fn vectors_round_trip_through_text() {
        assert_eq!(
            vector_literal(&[0.5, -1.0, 2.25]),
            "'[0.5,-1,2.25]'::vector"
        );
        assert_eq!(parse_vector("[0.5,-1,2.25]"), Some(vec![0.5, -1.0, 2.25]));
        assert_eq!(parse_vector("[]"), Some(vec![]));
        assert_eq!(parse_vector("0.5,1"), None);
        assert_eq!(parse_vector("[a]"), None);
    }

    #[test]
    fn conditions_read_nested_fields_and_quote_values() {
        let sql = condition("t", &cond("address.city", ConditionOp::Eq, json!("İzmir'"))).unwrap();
        assert!(sql.contains(r#"#> '{"address","city"}'"#), "{sql}");
        assert!(sql.contains(r#"'"İzmir''"'::jsonb"#), "{sql}");

        let sql = condition("t", &cond("year", ConditionOp::Gte, json!(2024))).unwrap();
        assert!(sql.contains("::numeric >= 2024"), "{sql}");

        let sql = condition("t", &cond("city", ConditionOp::In, json!(["a", "b"]))).unwrap();
        assert!(
            sql.starts_with(r#"('["a","b"]'::jsonb @> jsonb_build_array("#),
            "{sql}"
        );

        assert!(condition("t", &cond("city", ConditionOp::In, json!("a"))).is_err());
        assert!(condition("t", &cond("year", ConditionOp::Gt, json!(true))).is_err());
        assert_eq!(filter("t", &[]).unwrap(), "TRUE");
    }

    #[test]
    fn scores_turn_similarities_back_into_higher_is_better() {
        assert_eq!(Metric::Cosine.score("d"), "1 - (d)");
        assert_eq!(Metric::Dot.score("d"), "(d) * -1");
        assert_eq!(Metric::Euclid.score("d"), "d");
        assert!(Metric::Cosine.higher_is_better());
        assert!(!Metric::Manhattan.higher_is_better());
    }
}
