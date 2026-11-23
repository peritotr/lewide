//! Changing rows of a table: the statements, and applying them as one unit.

use serde::{Deserialize, Serialize};

use crate::sql::{SqlDialect, literal};
use crate::{Connection, Error, ObjectDescription, QueryRequest, Result, Value};

/// A column and the value it has or should get.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Cell {
    pub column: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RowChange {
    Insert {
        values: Vec<Cell>,
    },
    /// `key` holds the row's primary key as it is now.
    Update {
        key: Vec<Cell>,
        set: Vec<Cell>,
    },
    Delete {
        key: Vec<Cell>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChangeSet {
    #[serde(default)]
    pub namespace: Option<String>,
    pub table: String,
    pub changes: Vec<RowChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplyResult {
    /// The statements that ran, in order.
    pub statements: Vec<String>,
}

/// The statements for a change set, one per change.
///
/// Rows are addressed by their whole primary key and nothing else, so each
/// UPDATE or DELETE can match one row at most. A table without a primary key
/// can receive new rows but not edits.
pub fn statements(
    dialect: SqlDialect,
    table: &str,
    description: &ObjectDescription,
    changes: &[RowChange],
    quote: &dyn Fn(&str) -> String,
) -> Result<Vec<String>> {
    let column = |name: &str| -> Result<String> {
        if description.columns.iter().any(|c| c.name == name) {
            Ok(quote(name))
        } else {
            Err(Error::Query(format!("no such column: {name}")))
        }
    };
    let assign = |cell: &Cell| {
        Ok(format!(
            "{} = {}",
            column(&cell.column)?,
            literal(dialect, &cell.value)?
        ))
    };

    let primary: Vec<&str> = description
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    let condition = |key: &[Cell]| -> Result<String> {
        if primary.is_empty() {
            return Err(Error::Query(format!(
                "{} has no primary key, so its rows cannot be told apart for editing",
                description.name
            )));
        }
        let mut named: Vec<&str> = key.iter().map(|c| c.column.as_str()).collect();
        named.sort_unstable();
        let mut expected = primary.clone();
        expected.sort_unstable();
        if named != expected {
            return Err(Error::Query(format!(
                "a row is identified by its whole primary key ({})",
                primary.join(", ")
            )));
        }
        if key.iter().any(|c| c.value == Value::Null) {
            return Err(Error::Query("a primary key value cannot be NULL".into()));
        }
        Ok(key
            .iter()
            .map(&assign)
            .collect::<Result<Vec<_>>>()?
            .join(" AND "))
    };

    let mut out = Vec::new();
    let mut index = 0;
    while index < changes.len() {
        // Consecutive inserts into the same columns go to the server as one
        // statement: one round trip for many rows.
        if let RowChange::Insert { values } = &changes[index]
            && !values.is_empty()
        {
            let names: Vec<&str> = values.iter().map(|c| c.column.as_str()).collect();
            let mut group = vec![values];
            while group.len() < INSERT_GROUP_ROWS {
                match changes.get(index + group.len()) {
                    Some(RowChange::Insert { values: next })
                        if next.len() == names.len()
                            && next.iter().zip(&names).all(|(c, name)| c.column == *name) =>
                    {
                        group.push(next)
                    }
                    _ => break,
                }
            }
            if group.len() > 1 {
                let columns = names
                    .iter()
                    .map(|name| column(name))
                    .collect::<Result<Vec<_>>>()?;
                let rows = group
                    .iter()
                    .map(|values| {
                        let literals = values
                            .iter()
                            .map(|c| literal(dialect, &c.value))
                            .collect::<Result<Vec<_>>>()?;
                        Ok(format!("({})", literals.join(", ")))
                    })
                    .collect::<Result<Vec<_>>>()?;
                out.push(format!(
                    "INSERT INTO {table} ({}) VALUES {}",
                    columns.join(", "),
                    rows.join(", ")
                ));
                index += group.len();
                continue;
            }
        }
        let change = &changes[index];
        index += 1;
        out.push(match change {
            RowChange::Insert { values } if values.is_empty() => Ok(match dialect {
                SqlDialect::MySql => format!("INSERT INTO {table} () VALUES ()"),
                _ => format!("INSERT INTO {table} DEFAULT VALUES"),
            }),
            RowChange::Insert { values } => {
                let columns = values
                    .iter()
                    .map(|c| column(&c.column))
                    .collect::<Result<Vec<_>>>()?;
                let literals = values
                    .iter()
                    .map(|c| literal(dialect, &c.value))
                    .collect::<Result<Vec<_>>>()?;
                Ok(format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    columns.join(", "),
                    literals.join(", ")
                ))
            }
            RowChange::Update { set, .. } if set.is_empty() => {
                Err(Error::Query("an update has nothing to change".into()))
            }
            RowChange::Update { key, set } => {
                let assignments = set.iter().map(&assign).collect::<Result<Vec<_>>>()?;
                Ok(format!(
                    "UPDATE {table} SET {} WHERE {}",
                    assignments.join(", "),
                    condition(key)?
                ))
            }
            RowChange::Delete { key } => {
                Ok(format!("DELETE FROM {table} WHERE {}", condition(key)?))
            }
        }?);
    }
    Ok(out)
}

/// Rows written by one multi-row INSERT at most.
const INSERT_GROUP_ROWS: usize = 200;

/// Runs the change set on `conn` inside a transaction: either every change is
/// made or none is. A change that does not affect exactly one row (the row
/// was deleted or its key changed meanwhile) rolls everything back.
///
/// The connection must not have a transaction open: beginning one here would
/// end it. Callers check with [`Connection::in_transaction`] or use a
/// connection of their own.
pub async fn apply_on(conn: &dyn Connection, changes: &ChangeSet) -> Result<ApplyResult> {
    if changes.changes.is_empty() {
        return Ok(ApplyResult {
            statements: Vec::new(),
        });
    }
    let namespace = changes.namespace.as_deref();
    let description = conn.describe_object(namespace, &changes.table).await?;
    let table = conn.qualified(namespace, &changes.table);
    let statements = statements(
        conn.dialect(),
        &table,
        &description,
        &changes.changes,
        &|name| conn.quote_ident(name),
    )?;

    let run = |statement: String| {
        conn.execute(QueryRequest {
            statement,
            max_rows: None,
        })
    };
    run(conn.dialect().begin().into()).await?;
    // Which changes each statement holds, to check its row count and to say where a failure was.
    let mut spans = Vec::with_capacity(statements.len());
    let mut at = 0;
    for change in &changes.changes {
        match change {
            RowChange::Insert { values } if !values.is_empty() => {
                let names: Vec<&str> = values.iter().map(|c| c.column.as_str()).collect();
                let mut size = 1;
                while size < INSERT_GROUP_ROWS {
                    match changes.changes.get(at + size) {
                        Some(RowChange::Insert { values: next })
                            if next.len() == names.len()
                                && next.iter().zip(&names).all(|(c, name)| c.column == *name) =>
                        {
                            size += 1
                        }
                        _ => break,
                    }
                }
                if spans
                    .last()
                    .is_some_and(|(start, len): &(usize, usize)| start + len > at)
                {
                    // Still inside the previous group.
                } else {
                    spans.push((at, size));
                }
                at += 1;
            }
            _ => {
                spans.push((at, 1));
                at += 1;
            }
        }
    }
    for (statement, (start, size)) in statements.iter().zip(spans) {
        let place = if size == 1 {
            format!("change {}", start + 1)
        } else {
            format!("changes {} to {}", start + 1, start + size)
        };
        let failure = match run(statement.clone()).await {
            Ok(result) if result.rows_affected == Some(size as u64) => continue,
            Ok(result) => format!(
                "{place} affected {} rows instead of {size}; a row was probably changed or deleted by someone else",
                result.rows_affected.unwrap_or(0)
            ),
            Err(e) => format!("{place} failed: {e}"),
        };
        // Nothing is kept. If even the rollback fails the connection is broken, which the caller will see next.
        let _ = run("ROLLBACK".into()).await;
        return Err(Error::Query(format!(
            "{failure}. Nothing was applied.\n{statement}"
        )));
    }
    run("COMMIT".into()).await?;
    Ok(ApplyResult { statements })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_inserts_with_the_same_columns() {
        use crate::{ColumnDescription, Value};
        let column = |name: &str| ColumnDescription {
            name: name.into(),
            type_name: Some("int".into()),
            nullable: true,
            default_value: None,
            primary_key: name == "id",
            comment: None,
            enum_values: None,
        };
        let table = ObjectDescription {
            name: "t".into(),
            columns: vec![column("id"), column("a")],
        };
        let insert = |pairs: &[(&str, i64)]| RowChange::Insert {
            values: pairs
                .iter()
                .map(|(c, v)| Cell {
                    column: (*c).into(),
                    value: Value::Int(*v),
                })
                .collect(),
        };
        let changes = [
            insert(&[("id", 1), ("a", 2)]),
            insert(&[("id", 3), ("a", 4)]),
            insert(&[("id", 5)]),
            RowChange::Delete {
                key: vec![Cell {
                    column: "id".into(),
                    value: Value::Int(9),
                }],
            },
        ];
        let quote = |name: &str| format!("\"{name}\"");
        assert_eq!(
            statements(SqlDialect::Postgres, "t", &table, &changes, &quote).unwrap(),
            [
                "INSERT INTO t (\"id\", \"a\") VALUES (1, 2), (3, 4)",
                "INSERT INTO t (\"id\") VALUES (5)",
                "DELETE FROM t WHERE \"id\" = 9",
            ]
        );
    }
}
