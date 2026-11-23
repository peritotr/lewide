//! Changing a table's columns: the `ALTER TABLE` statements, and applying them.
//!
//! Type names and defaults are SQL the user wrote for their own database, and
//! go into the statements as they are. The statements are shown before they run.

use serde::Deserialize;

use crate::sql::SqlDialect;
use crate::{ApplyResult, Connection, Error, QueryRequest, Result};

/// A column as the user wants it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    /// The default expression as SQL, or none.
    #[serde(default)]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColumnChange {
    Add {
        column: ColumnDef,
    },
    /// `from` is the column as it was loaded; `to` is how it should be.
    Modify {
        from: ColumnDef,
        to: ColumnDef,
    },
    Drop {
        column: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StructureChangeSet {
    #[serde(default)]
    pub namespace: Option<String>,
    pub table: String,
    pub changes: Vec<ColumnChange>,
}

/// Whether the engine undoes DDL when a transaction is rolled back.
pub const fn transactional(dialect: SqlDialect) -> bool {
    !matches!(dialect, SqlDialect::MySql)
}

fn checked(text: &str, what: &str) -> Result<()> {
    // One fragment, one statement: a semicolon would start another.
    if text.contains(';') {
        return Err(Error::Query(format!("{what} cannot contain ';'")));
    }
    if text.trim().is_empty() {
        return Err(Error::Query(format!("{what} is empty")));
    }
    Ok(())
}

fn definition(dialect: SqlDialect, column: &ColumnDef) -> Result<String> {
    checked(&column.type_name, "a column type")?;
    let mut text = format!(
        "{} {}",
        dialect.quote_ident(&column.name),
        column.type_name.trim()
    );
    if let Some(default) = column
        .default_value
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        checked(default, "a default")?;
        text.push_str(&format!(" DEFAULT {}", default.trim()));
    }
    if !column.nullable {
        text.push_str(" NOT NULL");
    }
    Ok(text)
}

/// The statements for a change set: drops, then changes to existing columns, then new columns.
pub fn statements(
    dialect: SqlDialect,
    table: &str,
    changes: &[ColumnChange],
) -> Result<Vec<String>> {
    match dialect {
        SqlDialect::SqlServer => return sql_server(table, changes),
        // Its ALTER TABLE works differently again (MODIFY COLUMN, mutations).
        SqlDialect::ClickHouse => {
            return Err(Error::Unsupported(
                "changing columns is not built for ClickHouse yet",
            ));
        }
        _ => {}
    }
    let quote = |name: &str| dialect.quote_ident(name);
    let mut out = Vec::new();
    let ordered = |pick: fn(&ColumnChange) -> bool| changes.iter().filter(move |c| pick(c));

    for change in ordered(|c| matches!(c, ColumnChange::Drop { .. })) {
        let ColumnChange::Drop { column } = change else {
            unreachable!()
        };
        out.push(format!("ALTER TABLE {table} DROP COLUMN {}", quote(column)));
    }
    for change in ordered(|c| matches!(c, ColumnChange::Modify { .. })) {
        let ColumnChange::Modify { from, to } = change else {
            unreachable!()
        };
        checked(&to.name, "a column name")?;
        let type_changed = from.type_name.trim() != to.type_name.trim();
        let null_changed = from.nullable != to.nullable;
        let default_changed = from
            .default_value
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            != to
                .default_value
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
        let renamed = from.name != to.name;
        match dialect {
            SqlDialect::MySql => {
                if renamed || type_changed || null_changed || default_changed {
                    out.push(format!(
                        "ALTER TABLE {table} CHANGE COLUMN {} {}",
                        quote(&from.name),
                        definition(dialect, to)?
                    ));
                }
            }
            // DuckDB takes PostgreSQL's ALTER TABLE forms.
            SqlDialect::Postgres | SqlDialect::DuckDb => {
                if renamed {
                    out.push(format!(
                        "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                        quote(&from.name),
                        quote(&to.name)
                    ));
                }
                let name = quote(&to.name);
                if type_changed {
                    checked(&to.type_name, "a column type")?;
                    out.push(format!(
                        "ALTER TABLE {table} ALTER COLUMN {name} TYPE {}",
                        to.type_name.trim()
                    ));
                }
                if default_changed {
                    out.push(
                        match to
                            .default_value
                            .as_deref()
                            .map(str::trim)
                            .filter(|d| !d.is_empty())
                        {
                            Some(default) => {
                                checked(default, "a default")?;
                                format!(
                                    "ALTER TABLE {table} ALTER COLUMN {name} SET DEFAULT {default}"
                                )
                            }
                            None => format!("ALTER TABLE {table} ALTER COLUMN {name} DROP DEFAULT"),
                        },
                    );
                }
                if null_changed {
                    out.push(format!(
                        "ALTER TABLE {table} ALTER COLUMN {name} {} NOT NULL",
                        if to.nullable { "DROP" } else { "SET" }
                    ));
                }
            }
            SqlDialect::Sqlite => {
                if type_changed || null_changed || default_changed {
                    return Err(Error::Unsupported(
                        "SQLite cannot change a column's type, nullability or default; only its name",
                    ));
                }
                if renamed {
                    out.push(format!(
                        "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                        quote(&from.name),
                        quote(&to.name)
                    ));
                }
            }
            SqlDialect::SqlServer | SqlDialect::ClickHouse => unreachable!("handled at the top"),
        }
    }
    for change in ordered(|c| matches!(c, ColumnChange::Add { .. })) {
        let ColumnChange::Add { column } = change else {
            unreachable!()
        };
        out.push(format!(
            "ALTER TABLE {table} ADD COLUMN {}",
            definition(dialect, column)?
        ));
    }
    Ok(out)
}

/// A text as a T-SQL Unicode literal.
fn n_literal(text: &str) -> String {
    format!("N'{}'", text.replace('\'', "''"))
}

/// A batch that drops the column's default constraint, whatever the server
/// named it. T-SQL keeps a default as a constraint, and a column with one can
/// be neither dropped nor given another default before it is gone.
fn drop_default(table: &str, column: &str) -> String {
    format!(
        "DECLARE @default sysname = (SELECT dc.name FROM sys.default_constraints AS dc \
         JOIN sys.columns AS c ON c.object_id = dc.parent_object_id AND c.column_id = dc.parent_column_id \
         WHERE dc.parent_object_id = OBJECT_ID({}) AND c.name = {});\n\
         IF @default IS NOT NULL BEGIN \
         DECLARE @drop nvarchar(max) = N'ALTER TABLE {} DROP CONSTRAINT ' + QUOTENAME(@default); \
         EXEC (@drop) END",
        n_literal(table),
        n_literal(column),
        table.replace('\'', "''")
    )
}

/// T-SQL: `ADD` without `COLUMN`, `ALTER COLUMN` restating the type and
/// nullability together, `sp_rename` for names, and defaults as constraints.
fn sql_server(table: &str, changes: &[ColumnChange]) -> Result<Vec<String>> {
    let dialect = SqlDialect::SqlServer;
    let quote = |name: &str| dialect.quote_ident(name);
    let default_of = |column: &ColumnDef| {
        column
            .default_value
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
    };
    let mut out = Vec::new();
    for change in changes {
        if let ColumnChange::Drop { column } = change {
            out.push(drop_default(table, column));
            out.push(format!("ALTER TABLE {table} DROP COLUMN {}", quote(column)));
        }
    }
    for change in changes {
        let ColumnChange::Modify { from, to } = change else {
            continue;
        };
        checked(&to.name, "a column name")?;
        if from.name != to.name {
            out.push(format!(
                "EXEC sp_rename {}, {}, N'COLUMN'",
                n_literal(&format!("{table}.{}", quote(&from.name))),
                n_literal(&to.name)
            ));
        }
        let name = quote(&to.name);
        if from.type_name.trim() != to.type_name.trim() || from.nullable != to.nullable {
            checked(&to.type_name, "a column type")?;
            out.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {name} {} {}",
                to.type_name.trim(),
                if to.nullable { "NULL" } else { "NOT NULL" }
            ));
        }
        if default_of(from) != default_of(to) {
            out.push(drop_default(table, &to.name));
            if let Some(default) = default_of(to) {
                checked(&default, "a default")?;
                out.push(format!(
                    "ALTER TABLE {table} ADD DEFAULT {default} FOR {name}"
                ));
            }
        }
    }
    for change in changes {
        if let ColumnChange::Add { column } = change {
            out.push(format!(
                "ALTER TABLE {table} ADD {}",
                definition(dialect, column)?
            ));
        }
    }
    Ok(out)
}

/// Runs the statements in order. Where the engine can undo DDL they are one
/// transaction; elsewhere a failure leaves the earlier statements in place,
/// and the error says how many.
pub async fn apply_on(conn: &dyn Connection, statements: Vec<String>) -> Result<ApplyResult> {
    let run = |statement: &str| {
        conn.execute(QueryRequest {
            statement: statement.to_owned(),
            max_rows: None,
        })
    };
    let atomic = transactional(conn.dialect());
    if atomic {
        run(conn.dialect().begin()).await?;
    }
    let total = statements.len();
    for (index, statement) in statements.iter().enumerate() {
        if let Err(error) = run(statement).await {
            if atomic {
                let _ = run("ROLLBACK").await;
            }
            let tail = if atomic || index == 0 {
                "Nothing was changed.".to_owned()
            } else {
                format!("The {index} statements before it were applied and cannot be rolled back.")
            };
            return Err(Error::Query(format!(
                "statement {} of {total} failed: {error}. {tail}\n{statement}",
                index + 1
            )));
        }
    }
    if atomic {
        run("COMMIT").await?;
    }
    Ok(ApplyResult { statements })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(
        name: &str,
        type_name: &str,
        nullable: bool,
        default_value: Option<&str>,
    ) -> ColumnDef {
        ColumnDef {
            name: name.into(),
            type_name: type_name.into(),
            nullable,
            default_value: default_value.map(str::to_owned),
        }
    }

    fn set(changes: Vec<ColumnChange>) -> StructureChangeSet {
        StructureChangeSet {
            namespace: None,
            table: "orders".into(),
            changes,
        }
    }

    fn sql(dialect: SqlDialect, table: &str, changes: Vec<ColumnChange>) -> Vec<String> {
        statements(dialect, table, &set(changes).changes).unwrap()
    }

    #[test]
    fn postgres_changes_each_property_on_its_own() {
        let from = column("note", "text", true, None);
        let to = column("remark", "varchar(200)", false, Some("''"));
        assert_eq!(
            sql(
                SqlDialect::Postgres,
                "\"shop\".\"orders\"",
                vec![ColumnChange::Modify { from, to }]
            ),
            [
                "ALTER TABLE \"shop\".\"orders\" RENAME COLUMN \"note\" TO \"remark\"",
                "ALTER TABLE \"shop\".\"orders\" ALTER COLUMN \"remark\" TYPE varchar(200)",
                "ALTER TABLE \"shop\".\"orders\" ALTER COLUMN \"remark\" SET DEFAULT ''",
                "ALTER TABLE \"shop\".\"orders\" ALTER COLUMN \"remark\" SET NOT NULL",
            ]
        );
    }

    #[test]
    fn sql_server_uses_t_sql_forms() {
        let table = "[dbo].[orders]";
        let changes = vec![
            ColumnChange::Drop {
                column: "old".into(),
            },
            ColumnChange::Modify {
                from: column("note", "nvarchar(50)", true, Some("N'x'")),
                to: column("remark", "nvarchar(200)", false, None),
            },
            ColumnChange::Add {
                column: column("qty", "int", false, Some("0")),
            },
        ];
        let out = sql(SqlDialect::SqlServer, table, changes);
        assert_eq!(out.len(), 6);
        assert!(
            out[0].contains("OBJECT_ID(N'[dbo].[orders]')") && out[0].contains("c.name = N'old'")
        );
        assert_eq!(out[1], "ALTER TABLE [dbo].[orders] DROP COLUMN [old]");
        assert_eq!(
            out[2],
            "EXEC sp_rename N'[dbo].[orders].[note]', N'remark', N'COLUMN'"
        );
        assert_eq!(
            out[3],
            "ALTER TABLE [dbo].[orders] ALTER COLUMN [remark] nvarchar(200) NOT NULL"
        );
        assert!(out[4].contains("c.name = N'remark'"));
        assert_eq!(
            out[5],
            "ALTER TABLE [dbo].[orders] ADD [qty] int DEFAULT 0 NOT NULL"
        );

        // A new default replaces the old constraint; a name with a quote stays one literal.
        let out = sql(
            SqlDialect::SqlServer,
            "[dbo].[o'k]",
            vec![ColumnChange::Modify {
                from: column("n", "int", true, None),
                to: column("n", "int", true, Some("1")),
            }],
        );
        assert_eq!(out.len(), 2);
        assert!(
            out[0].contains("OBJECT_ID(N'[dbo].[o''k]')")
                && out[0].contains("ALTER TABLE [dbo].[o''k] DROP CONSTRAINT")
        );
        assert_eq!(out[1], "ALTER TABLE [dbo].[o'k] ADD DEFAULT 1 FOR [n]");
    }

    #[test]
    fn drops_come_first_and_additions_last() {
        let out = sql(
            SqlDialect::Postgres,
            "t",
            vec![
                ColumnChange::Add {
                    column: column("b", "int", false, Some("0")),
                },
                ColumnChange::Drop { column: "a".into() },
            ],
        );
        assert_eq!(
            out,
            [
                "ALTER TABLE t DROP COLUMN \"a\"",
                "ALTER TABLE t ADD COLUMN \"b\" int DEFAULT 0 NOT NULL"
            ]
        );
    }

    #[test]
    fn mysql_restates_the_whole_column() {
        let from = column("note", "text", true, None);
        let to = column("note", "varchar(200)", false, None);
        assert_eq!(
            sql(
                SqlDialect::MySql,
                "`orders`",
                vec![ColumnChange::Modify {
                    from: from.clone(),
                    to
                }]
            ),
            ["ALTER TABLE `orders` CHANGE COLUMN `note` `note` varchar(200) NOT NULL"]
        );
        // Nothing changed: nothing to run.
        assert!(
            sql(
                SqlDialect::MySql,
                "`orders`",
                vec![ColumnChange::Modify {
                    from: from.clone(),
                    to: from
                }]
            )
            .is_empty()
        );
    }

    #[test]
    fn sqlite_renames_but_does_not_retype() {
        let from = column("note", "text", true, None);
        assert_eq!(
            sql(
                SqlDialect::Sqlite,
                "\"orders\"",
                vec![ColumnChange::Modify {
                    from: from.clone(),
                    to: column("remark", "text", true, None)
                }]
            ),
            ["ALTER TABLE \"orders\" RENAME COLUMN \"note\" TO \"remark\""]
        );
        assert!(
            statements(
                SqlDialect::Sqlite,
                "\"orders\"",
                &[ColumnChange::Modify {
                    from,
                    to: column("note", "int", true, None)
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn a_semicolon_in_a_type_or_default_is_refused() {
        assert!(
            statements(
                SqlDialect::Postgres,
                "t",
                &[ColumnChange::Add {
                    column: column("a", "int; DROP TABLE t", true, None)
                }]
            )
            .is_err()
        );
        assert!(
            statements(
                SqlDialect::Postgres,
                "t",
                &[ColumnChange::Add {
                    column: column("a", "int", true, Some("0; DROP TABLE t"))
                }]
            )
            .is_err()
        );
        assert!(
            statements(
                SqlDialect::Postgres,
                "t",
                &[ColumnChange::Add {
                    column: column("a", " ", true, None)
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn groups_inserts_with_the_same_columns() {
        let def = |name: &str| ColumnDef {
            name: name.into(),
            type_name: "int".into(),
            nullable: true,
            default_value: None,
        };
        let add = |name: &str| ColumnChange::Add { column: def(name) };
        assert_eq!(
            statements(
                SqlDialect::Postgres,
                "t",
                &[
                    add("b"),
                    add("c"),
                    ColumnChange::Drop { column: "a".into() }
                ]
            )
            .unwrap(),
            [
                "ALTER TABLE t DROP COLUMN \"a\"",
                "ALTER TABLE t ADD COLUMN \"b\" int",
                "ALTER TABLE t ADD COLUMN \"c\" int",
            ]
        );
        let renamed = ColumnChange::Modify {
            from: column("id", "int", true, None),
            to: column("key", "bigint", false, Some("0")),
        };
        assert_eq!(
            statements(SqlDialect::Postgres, "t", std::slice::from_ref(&renamed)).unwrap(),
            [
                "ALTER TABLE t RENAME COLUMN \"id\" TO \"key\"",
                "ALTER TABLE t ALTER COLUMN \"key\" TYPE bigint",
                "ALTER TABLE t ALTER COLUMN \"key\" SET DEFAULT 0",
                "ALTER TABLE t ALTER COLUMN \"key\" SET NOT NULL",
            ]
        );
        assert_eq!(
            statements(SqlDialect::MySql, "t", &[renamed]).unwrap(),
            ["ALTER TABLE t CHANGE COLUMN `id` `key` bigint DEFAULT 0 NOT NULL"]
        );
    }
}
