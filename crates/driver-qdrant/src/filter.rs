//! Turns the neutral payload conditions into Qdrant's filter JSON.
//!
//! Every condition becomes exactly one entry of the filter's `must` list, so
//! the conditions are joined with AND. Negations are nested filters
//! (`{"must_not": [...]}`), which Qdrant accepts wherever a condition goes.

use dbclient_core::vector::{Condition, ConditionOp};
use dbclient_core::{Error, Result};
use serde_json::{Value, json};

/// Whole numbers up to here are exact as floating point, which is what a
/// range comparison uses.
const EXACT_AS_FLOAT: u64 = 1 << 53;

/// `None` when there is nothing to filter on.
pub fn filter(conditions: &[Condition]) -> Result<Option<Value>> {
    if conditions.is_empty() {
        return Ok(None);
    }
    let must = conditions
        .iter()
        .map(condition)
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(json!({ "must": must })))
}

fn condition(condition: &Condition) -> Result<Value> {
    let field = condition.field.trim();
    if field.is_empty() {
        return Err(Error::Query("a filter condition needs a field".into()));
    }
    let value = &condition.value;
    Ok(match condition.op {
        ConditionOp::Eq => equals(field, value)?,
        ConditionOp::Ne => json!({ "must_not": [equals(field, value)?] }),
        ConditionOp::Gt => range(field, "gt", value)?,
        ConditionOp::Gte => range(field, "gte", value)?,
        ConditionOp::Lt => range(field, "lt", value)?,
        ConditionOp::Lte => range(field, "lte", value)?,
        ConditionOp::In => {
            let Value::Array(items) = value else {
                return Err(Error::Query(format!("{field}: IN needs a list of values")));
            };
            // An empty list goes this way too: `any: []` matches nothing,
            // while an empty `should` would match everything.
            if items.iter().all(Value::is_string) {
                json!({ "key": field, "match": { "any": items } })
            } else {
                let any = items
                    .iter()
                    .map(|item| equals(field, item))
                    .collect::<Result<Vec<_>>>()?;
                json!({ "should": any })
            }
        }
        // `is_empty` covers a missing field, a null and an empty list.
        // Qdrant's `is_null` would only find fields stored as an explicit null.
        ConditionOp::IsNull => json!({ "is_empty": { "key": field } }),
        ConditionOp::IsNotNull => json!({ "must_not": [{ "is_empty": { "key": field } }] }),
    })
}

fn equals(field: &str, value: &Value) -> Result<Value> {
    Ok(match value {
        Value::String(_) | Value::Bool(_) => json!({ "key": field, "match": { "value": value } }),
        Value::Number(number) => {
            let exact = match (number.as_u64(), number.as_i64()) {
                (Some(n), _) => n <= EXACT_AS_FLOAT,
                (None, Some(n)) => n.unsigned_abs() <= EXACT_AS_FLOAT,
                (None, None) => true,
            };
            if exact {
                // `match` takes whole numbers only and does not find 5 stored
                // as 5.0. A range from the value to itself finds both.
                json!({ "key": field, "range": { "gte": value, "lte": value } })
            } else {
                // Too large to compare as floating point without rounding.
                json!({ "key": field, "match": { "value": value } })
            }
        }
        Value::Null => {
            return Err(Error::Query(format!(
                "{field}: cannot compare with null; use \"is null\""
            )));
        }
        Value::Array(_) | Value::Object(_) => {
            return Err(Error::Query(format!(
                "{field}: can only compare with text, a number or true/false"
            )));
        }
    })
}

fn range(field: &str, bound: &str, value: &Value) -> Result<Value> {
    match value {
        // Text bounds are date-times; the server checks the format and says so.
        Value::Number(_) | Value::String(_) => {
            Ok(json!({ "key": field, "range": { bound: value } }))
        }
        _ => Err(Error::Query(format!(
            "{field}: can only order by a number or a date-time"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(op: ConditionOp, value: Value) -> Result<Value> {
        let conditions = [Condition {
            field: "f".into(),
            op,
            value,
        }];
        filter(&conditions).map(|f| f.unwrap()["must"][0].clone())
    }

    #[test]
    fn no_conditions_no_filter() {
        assert_eq!(filter(&[]).unwrap(), None);
    }

    #[test]
    fn conditions_are_joined_with_and() {
        let conditions = [
            Condition {
                field: "city".into(),
                op: ConditionOp::Eq,
                value: json!("Berlin"),
            },
            Condition {
                field: "n".into(),
                op: ConditionOp::Gt,
                value: json!(3),
            },
        ];
        assert_eq!(
            filter(&conditions).unwrap().unwrap(),
            json!({ "must": [
                { "key": "city", "match": { "value": "Berlin" } },
                { "key": "n", "range": { "gt": 3 } }
            ]})
        );
    }

    #[test]
    fn each_operator() {
        use ConditionOp::*;
        assert_eq!(
            one(Eq, json!(true)).unwrap(),
            json!({ "key": "f", "match": { "value": true } })
        );
        assert_eq!(
            one(Eq, json!(2.5)).unwrap(),
            json!({ "key": "f", "range": { "gte": 2.5, "lte": 2.5 } })
        );
        assert_eq!(
            one(Eq, json!(u64::MAX)).unwrap(),
            json!({ "key": "f", "match": { "value": u64::MAX } })
        );
        assert_eq!(
            one(Ne, json!("a")).unwrap(),
            json!({ "must_not": [{ "key": "f", "match": { "value": "a" } }] })
        );
        assert_eq!(
            one(Gte, json!(1)).unwrap(),
            json!({ "key": "f", "range": { "gte": 1 } })
        );
        assert_eq!(
            one(Lt, json!("2024-01-01T00:00:00Z")).unwrap(),
            json!({ "key": "f", "range": { "lt": "2024-01-01T00:00:00Z" } })
        );
        assert_eq!(
            one(Lte, json!(1)).unwrap(),
            json!({ "key": "f", "range": { "lte": 1 } })
        );
        assert_eq!(
            one(In, json!(["a", "b"])).unwrap(),
            json!({ "key": "f", "match": { "any": ["a", "b"] } })
        );
        assert_eq!(
            one(In, json!([])).unwrap(),
            json!({ "key": "f", "match": { "any": [] } })
        );
        assert_eq!(
            one(In, json!([1, "a"])).unwrap(),
            json!({ "should": [
                { "key": "f", "range": { "gte": 1, "lte": 1 } },
                { "key": "f", "match": { "value": "a" } }
            ]})
        );
        assert_eq!(
            one(IsNull, Value::Null).unwrap(),
            json!({ "is_empty": { "key": "f" } })
        );
        assert_eq!(
            one(IsNotNull, Value::Null).unwrap(),
            json!({ "must_not": [{ "is_empty": { "key": "f" } }] })
        );
    }

    #[test]
    fn values_that_cannot_be_compared_are_refused() {
        use ConditionOp::*;
        for (op, value) in [
            (Eq, Value::Null),
            (Ne, json!({})),
            (Gt, json!(true)),
            (In, json!("a")),
            (In, json!([null])),
        ] {
            assert!(
                matches!(one(op, value.clone()), Err(Error::Query(_))),
                "{op:?} {value}"
            );
        }
        let blank = [Condition {
            field: " ".into(),
            op: IsNull,
            value: Value::Null,
        }];
        assert!(matches!(filter(&blank), Err(Error::Query(_))));
    }
}
