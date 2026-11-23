//! Turns the vector view's payload conditions into a Milvus boolean expression.
//!
//! The view's rules are those of Qdrant: when a field holds a list, a
//! condition holds if one item satisfies it, and `Ne` also matches points
//! without the field. Milvus does neither by itself, so array fields use its
//! `array_contains` functions and `Ne` on a field that can be missing adds
//! `not exists`.

use dbclient_core::vector::{Condition, ConditionOp};
use dbclient_core::{Error, Result};
use serde_json::Value;

/// What the expression needs to know about one schema field.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    /// Milvus's type name: `Int64`, `VarChar`, `Array`, `JSON`...
    pub kind: String,
    pub nullable: bool,
}

/// The fields of a collection that conditions can refer to.
pub struct Schema<'a> {
    pub fields: &'a [Field],
    /// Whether keys outside the schema are kept (in `$meta`).
    pub dynamic: bool,
}

/// All conditions joined with `and`; `None` when there are none.
pub fn filter(conditions: &[Condition], schema: &Schema) -> Result<Option<String>> {
    let parts = conditions
        .iter()
        .map(|c| condition(c, schema))
        .collect::<Result<Vec<_>>>()?;
    Ok(match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => Some(
            parts
                .iter()
                .map(|p| format!("({p})"))
                .collect::<Vec<_>>()
                .join(" and "),
        ),
    })
}

/// Where a condition's field lives.
enum Target<'a> {
    /// A schema field that is not a list.
    Plain { path: String, field: &'a Field },
    /// A schema field of type `Array`.
    List { name: &'a str },
    /// A key inside a JSON field or `$meta`: it may be missing.
    Json { path: String },
}

fn target<'a>(name: &str, schema: &'a Schema) -> Result<Target<'a>> {
    let mut parts = name.split('.');
    let first = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let keys = |prefix: String, keys: &[&str]| {
        keys.iter()
            .fold(prefix, |path, key| format!("{path}[{}]", string(key)))
    };
    match schema.fields.iter().find(|f| f.name == first) {
        Some(field) if field.kind == "JSON" => Ok(Target::Json {
            path: keys(first.to_owned(), &rest),
        }),
        Some(field) if rest.is_empty() && field.kind == "Array" => {
            Ok(Target::List { name: &field.name })
        }
        Some(field) if rest.is_empty() => Ok(Target::Plain {
            path: field.name.clone(),
            field,
        }),
        Some(_) => Err(Error::Query(format!("{first} has no fields inside it"))),
        None if schema.dynamic => Ok(Target::Json {
            path: keys("$meta".to_owned(), &name.split('.').collect::<Vec<_>>()),
        }),
        None => Err(Error::Query(format!("the collection has no field {first}"))),
    }
}

fn condition(c: &Condition, schema: &Schema) -> Result<String> {
    if c.field.trim().is_empty() {
        return Err(Error::Query("a condition has no field".into()));
    }
    let target = target(c.field.trim(), schema)?;
    let op = match c.op {
        ConditionOp::Eq => "==",
        ConditionOp::Ne => "!=",
        ConditionOp::Gt => ">",
        ConditionOp::Gte => ">=",
        ConditionOp::Lt => "<",
        ConditionOp::Lte => "<=",
        ConditionOp::In | ConditionOp::IsNull | ConditionOp::IsNotNull => "",
    };
    Ok(match (&target, c.op) {
        (Target::List { name }, ConditionOp::IsNull) => format!("array_length({name}) == 0"),
        (Target::List { name }, ConditionOp::IsNotNull) => format!("array_length({name}) > 0"),
        (Target::List { name }, ConditionOp::Eq) => {
            format!("array_contains({name}, {})", scalar(&c.value)?)
        }
        (Target::List { name }, ConditionOp::Ne) => {
            format!("not array_contains({name}, {})", scalar(&c.value)?)
        }
        (Target::List { name }, ConditionOp::In) => {
            format!("array_contains_any({name}, {})", list(&c.value)?)
        }
        (Target::List { .. }, _) => {
            return Err(Error::Query(format!(
                "{} holds a list; Milvus cannot compare its items with < or >",
                c.field
            )));
        }

        (Target::Json { path }, ConditionOp::IsNull) => format!("not exists {path}"),
        (Target::Json { path }, ConditionOp::IsNotNull) => format!("exists {path}"),
        (Target::Json { path }, ConditionOp::Ne) => {
            format!("not exists {path} or {path} != {}", scalar(&c.value)?)
        }
        (Target::Json { path }, ConditionOp::In) => format!("{path} in {}", list(&c.value)?),
        (Target::Json { path }, _) => format!("{path} {op} {}", scalar(&c.value)?),

        (Target::Plain { path, field }, ConditionOp::IsNull) => {
            if field.nullable {
                format!("{path} is null")
            } else {
                // A field that cannot be empty: no point matches.
                format!("{path} is null and {path} is not null")
            }
        }
        (Target::Plain { path, field }, ConditionOp::IsNotNull) => {
            if field.nullable {
                format!("{path} is not null")
            } else {
                format!("not ({path} is null)")
            }
        }
        (Target::Plain { path, field }, ConditionOp::Ne) if field.nullable => {
            format!("{path} is null or {path} != {}", scalar(&c.value)?)
        }
        (Target::Plain { path, .. }, ConditionOp::In) => format!("{path} in {}", list(&c.value)?),
        (Target::Plain { path, .. }, _) => format!("{path} {op} {}", scalar(&c.value)?),
    })
}

/// A string literal. Milvus reads JSON's escapes, so JSON's quoting is used.
pub fn string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(string(text)),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        _ => Err(Error::Query(format!(
            "a condition compares with a text, number or true/false, not {value}"
        ))),
    }
}

fn list(value: &Value) -> Result<String> {
    let Value::Array(items) = value else {
        return Err(Error::Query("\"in\" needs a list of values".into()));
    };
    if items.is_empty() {
        return Err(Error::Query("\"in\" needs at least one value".into()));
    }
    let items = items.iter().map(scalar).collect::<Result<Vec<_>>>()?;
    Ok(format!("[{}]", items.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> Vec<Field> {
        let field = |name: &str, kind: &str, nullable| Field {
            name: name.into(),
            kind: kind.into(),
            nullable,
        };
        vec![
            field("id", "Int64", false),
            field("title", "VarChar", false),
            field("note", "VarChar", true),
            field("tags", "Array", false),
            field("meta", "JSON", true),
        ]
    }

    fn one(field: &str, op: ConditionOp, value: Value) -> Result<Option<String>> {
        let fields = fields();
        let schema = Schema {
            fields: &fields,
            dynamic: true,
        };
        filter(
            &[Condition {
                field: field.into(),
                op,
                value,
            }],
            &schema,
        )
    }

    fn text(field: &str, op: ConditionOp, value: Value) -> String {
        one(field, op, value).unwrap().unwrap()
    }

    #[test]
    fn schema_fields() {
        assert_eq!(
            text("title", ConditionOp::Eq, json!("Çağrı \"x\"")),
            r#"title == "Çağrı \"x\"""#
        );
        assert_eq!(text("id", ConditionOp::Gte, json!(5)), "id >= 5");
        assert_eq!(text("id", ConditionOp::In, json!([1, 2])), "id in [1, 2]");
        assert_eq!(
            text("note", ConditionOp::Ne, json!("a")),
            r#"note is null or note != "a""#
        );
        assert_eq!(
            text("title", ConditionOp::Ne, json!("a")),
            r#"title != "a""#
        );
        assert_eq!(
            text("note", ConditionOp::IsNull, Value::Null),
            "note is null"
        );
    }

    #[test]
    fn lists_match_when_an_item_does() {
        assert_eq!(
            text("tags", ConditionOp::Eq, json!("t1")),
            r#"array_contains(tags, "t1")"#
        );
        assert_eq!(
            text("tags", ConditionOp::In, json!(["a", "b"])),
            r#"array_contains_any(tags, ["a", "b"])"#
        );
        assert_eq!(
            text("tags", ConditionOp::IsNull, Value::Null),
            "array_length(tags) == 0"
        );
        assert!(one("tags", ConditionOp::Gt, json!(1)).is_err());
    }

    #[test]
    fn keys_outside_the_schema_may_be_missing() {
        assert_eq!(
            text("city", ConditionOp::Eq, json!("İzmir")),
            r#"$meta["city"] == "İzmir""#
        );
        assert_eq!(
            text("address.city", ConditionOp::Ne, json!("x")),
            r#"not exists $meta["address"]["city"] or $meta["address"]["city"] != "x""#
        );
        assert_eq!(
            text("meta.a", ConditionOp::IsNotNull, Value::Null),
            r#"exists meta["a"]"#
        );
        assert!(one("title.x", ConditionOp::Eq, json!(1)).is_err());
    }

    #[test]
    fn several_conditions_are_joined() {
        let fields = fields();
        let schema = Schema {
            fields: &fields,
            dynamic: false,
        };
        let conditions = [
            Condition {
                field: "id".into(),
                op: ConditionOp::Gt,
                value: json!(1),
            },
            Condition {
                field: "title".into(),
                op: ConditionOp::Eq,
                value: json!("a"),
            },
        ];
        assert_eq!(
            filter(&conditions, &schema).unwrap().unwrap(),
            r#"(id > 1) and (title == "a")"#
        );
        assert_eq!(filter(&[], &schema).unwrap(), None);
        // Without dynamic fields an unknown name is an error, not an empty result.
        let unknown = [Condition {
            field: "city".into(),
            op: ConditionOp::Eq,
            value: json!("a"),
        }];
        assert!(filter(&unknown, &schema).is_err());
    }
}
