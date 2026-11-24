//! Changing a table's structure (columns, indexes, foreign keys, triggers):
//! the statements, and applying them.
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
    AddIndex {
        name: String,
        /// In key order.
        columns: Vec<IndexColumn>,
        #[serde(default)]
        unique: bool,
    },
    DropIndex {
        name: String,
    },
    AddForeignKey {
        name: String,
        columns: Vec<String>,
        /// The referenced table's namespace; `None` for the table's own.
        #[serde(default)]
        ref_namespace: Option<String>,
        ref_table: String,
        ref_columns: Vec<String>,
        /// `CASCADE`, `SET NULL`, `SET DEFAULT`, `RESTRICT` or `NO ACTION`.
        #[serde(default)]
        on_delete: Option<String>,
        #[serde(default)]
        on_update: Option<String>,
    },
    DropForeignKey {
        name: String,
    },
    DropTrigger {
        name: String,
    },
    /// A trigger as the user wrote it: the whole `CREATE TRIGGER` text, with
    /// whatever the engine needs beside it (PostgreSQL's trigger function).
    /// It runs as one request, so the engine's own parser reads the body and
    /// its semicolons. Editing a trigger is a drop of the old one and this.
    CreateTrigger {
        name: String,
        definition: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IndexColumn {
    pub name: String,
    #[serde(default)]
    pub descending: bool,
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
    !matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse)
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

/// The statements for a change set, in an order that cannot trip over itself:
/// foreign keys, triggers and indexes are dropped first, then columns are
/// dropped, changed and added, and new indexes and foreign keys come last.
/// `table` is the table's quoted, qualified name; `namespace` its namespace.
pub fn statements(
    dialect: SqlDialect,
    namespace: Option<&str>,
    table: &str,
    changes: &[ColumnChange],
) -> Result<Vec<String>> {
    if dialect == SqlDialect::ClickHouse
        && changes.iter().any(|c| {
            !matches!(
                c,
                ColumnChange::Add { .. } | ColumnChange::Modify { .. } | ColumnChange::Drop { .. }
            )
        })
    {
        // ClickHouse has no foreign keys or triggers, and its indexes are
        // data-skipping indexes with a type and granularity of their own.
        return Err(Error::Unsupported(
            "ClickHouse changes columns only; its indexes are written as SQL",
        ));
    }
    let (columns, others): (Vec<ColumnChange>, Vec<ColumnChange>) =
        changes.iter().cloned().partition(|c| {
            matches!(
                c,
                ColumnChange::Add { .. } | ColumnChange::Modify { .. } | ColumnChange::Drop { .. }
            )
        });
    let mut out = Vec::new();
    for change in &others {
        if !matches!(
            change,
            ColumnChange::AddIndex { .. }
                | ColumnChange::AddForeignKey { .. }
                | ColumnChange::CreateTrigger { .. }
        ) {
            out.push(object_statement(dialect, namespace, table, change)?);
        }
    }
    if !columns.is_empty() {
        out.extend(column_statements(dialect, table, &columns)?);
    }
    for change in &others {
        if matches!(change, ColumnChange::AddIndex { .. }) {
            out.push(object_statement(dialect, namespace, table, change)?);
        }
    }
    for change in &others {
        if matches!(change, ColumnChange::AddForeignKey { .. }) {
            out.push(object_statement(dialect, namespace, table, change)?);
        }
    }
    // Last: a trigger may name the new columns.
    for change in &others {
        if matches!(change, ColumnChange::CreateTrigger { .. }) {
            out.push(object_statement(dialect, namespace, table, change)?);
        }
    }
    Ok(out)
}

/// A name inside the table's namespace: `"ns"."name"`, or the name alone.
fn in_namespace(dialect: SqlDialect, namespace: Option<&str>, name: &str) -> String {
    match namespace.filter(|n| !n.is_empty()) {
        Some(ns) => format!("{}.{}", dialect.quote_ident(ns), dialect.quote_ident(name)),
        None => dialect.quote_ident(name),
    }
}

/// What a foreign key does when the referenced row goes away or changes.
fn referential_action(action: &Option<String>) -> Result<Option<&'static str>> {
    let Some(action) = action.as_deref().map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(None);
    };
    const ACTIONS: [&str; 5] = [
        "CASCADE",
        "SET NULL",
        "SET DEFAULT",
        "RESTRICT",
        "NO ACTION",
    ];
    let normal = action
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    ACTIONS
        .iter()
        .find(|a| **a == normal)
        .copied()
        .map(Some)
        .ok_or_else(|| Error::Query(format!("not a referential action: {action}")))
}

/// One statement for an index, foreign key or trigger change.
fn object_statement(
    dialect: SqlDialect,
    namespace: Option<&str>,
    table: &str,
    change: &ColumnChange,
) -> Result<String> {
    let quote = |name: &str| dialect.quote_ident(name);
    let named = |name: &str, what: &str| -> Result<()> {
        if name.trim().is_empty() {
            return Err(Error::Query(format!("the {what} needs a name")));
        }
        Ok(())
    };
    let list = |names: &[String], what: &str| -> Result<String> {
        if names.is_empty() {
            return Err(Error::Query(format!("{what} needs at least one column")));
        }
        Ok(names
            .iter()
            .map(|n| quote(n))
            .collect::<Vec<_>>()
            .join(", "))
    };
    Ok(match change {
        ColumnChange::AddIndex {
            name,
            columns,
            unique,
        } => {
            named(name, "index")?;
            if columns.is_empty() {
                return Err(Error::Query("an index needs at least one column".into()));
            }
            let keys = columns
                .iter()
                .map(|c| {
                    format!(
                        "{}{}",
                        quote(&c.name),
                        if c.descending { " DESC" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "CREATE {}INDEX {} ON {table} ({keys})",
                if *unique { "UNIQUE " } else { "" },
                quote(name)
            )
        }
        ColumnChange::DropIndex { name } => match dialect {
            SqlDialect::MySql | SqlDialect::SqlServer => {
                format!("DROP INDEX {} ON {table}", quote(name))
            }
            // An index lives in the schema, not under the table.
            _ => format!("DROP INDEX {}", in_namespace(dialect, namespace, name)),
        },
        ColumnChange::AddForeignKey {
            name,
            columns,
            ref_namespace,
            ref_table,
            ref_columns,
            on_delete,
            on_update,
        } => {
            if matches!(dialect, SqlDialect::Sqlite | SqlDialect::DuckDb) {
                return Err(Error::Unsupported(
                    "SQLite and DuckDB add a foreign key only when the table is created",
                ));
            }
            named(name, "foreign key")?;
            if columns.len() != ref_columns.len() {
                return Err(Error::Query(
                    "a foreign key needs as many referenced columns as columns".into(),
                ));
            }
            let target = in_namespace(dialect, ref_namespace.as_deref().or(namespace), ref_table);
            let mut text = format!(
                "ALTER TABLE {table} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {target} ({})",
                quote(name),
                list(columns, "a foreign key")?,
                list(ref_columns, "a foreign key")?
            );
            if let Some(action) = referential_action(on_delete)? {
                text.push_str(&format!(" ON DELETE {action}"));
            }
            if let Some(action) = referential_action(on_update)? {
                text.push_str(&format!(" ON UPDATE {action}"));
            }
            text
        }
        ColumnChange::DropForeignKey { name } => match dialect {
            SqlDialect::MySql => format!("ALTER TABLE {table} DROP FOREIGN KEY {}", quote(name)),
            SqlDialect::Sqlite | SqlDialect::DuckDb => {
                return Err(Error::Unsupported(
                    "SQLite and DuckDB drop a foreign key only by creating the table again",
                ));
            }
            _ => format!("ALTER TABLE {table} DROP CONSTRAINT {}", quote(name)),
        },
        ColumnChange::DropTrigger { name } => match dialect {
            SqlDialect::Postgres => format!("DROP TRIGGER {} ON {table}", quote(name)),
            SqlDialect::Sqlite => format!("DROP TRIGGER {}", quote(name)),
            SqlDialect::DuckDb => return Err(Error::Unsupported("DuckDB has no triggers")),
            _ => format!("DROP TRIGGER {}", in_namespace(dialect, namespace, name)),
        },
        ColumnChange::CreateTrigger { name, definition } => {
            named(name, "trigger")?;
            if dialect == SqlDialect::DuckDb {
                return Err(Error::Unsupported("DuckDB has no triggers"));
            }
            let text = definition.trim().trim_end_matches(';').trim_end();
            if text.is_empty() {
                return Err(Error::Query("the trigger has no definition".into()));
            }
            text.to_owned()
        }
        ColumnChange::Add { .. } | ColumnChange::Modify { .. } | ColumnChange::Drop { .. } => {
            unreachable!("column changes are built elsewhere")
        }
    })
}

/// The statements for column changes: drops, then changes to existing columns, then new columns.
fn column_statements(
    dialect: SqlDialect,
    table: &str,
    changes: &[ColumnChange],
) -> Result<Vec<String>> {
    match dialect {
        SqlDialect::SqlServer => return sql_server(table, changes),
        SqlDialect::ClickHouse => return clickhouse(table, changes),
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

/// A ClickHouse type with the nullability asked for. ClickHouse keeps NULL in
/// the type (`Nullable(String)`, `LowCardinality(Nullable(String))`), so the
/// switch in the form wraps or unwraps it rather than adding `NOT NULL`.
fn clickhouse_type(type_name: &str, nullable: bool) -> String {
    let ty = type_name.trim();
    let inner = |text: &str, prefix: &str| {
        text.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(')'))
            .map(str::to_owned)
    };
    if let Some(low) = inner(ty, "LowCardinality(") {
        let base = inner(&low, "Nullable(").unwrap_or(low);
        return if nullable {
            format!("LowCardinality(Nullable({base}))")
        } else {
            format!("LowCardinality({base})")
        };
    }
    let base = inner(ty, "Nullable(").unwrap_or_else(|| ty.to_owned());
    if nullable {
        format!("Nullable({base})")
    } else {
        base
    }
}

/// A ClickHouse default: `DEFAULT expr`, or the kind as the catalog shows it
/// (`MATERIALIZED expr`, `ALIAS expr`, `EPHEMERAL expr`).
fn clickhouse_default(default: &str) -> Result<String> {
    checked(default, "a default")?;
    let text = default.trim();
    let kind = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    Ok(
        if matches!(
            kind.as_str(),
            "DEFAULT" | "MATERIALIZED" | "ALIAS" | "EPHEMERAL"
        ) {
            text.to_owned()
        } else {
            format!("DEFAULT {text}")
        },
    )
}

/// ClickHouse: `MODIFY COLUMN` for types and defaults, `REMOVE DEFAULT`,
/// `RENAME COLUMN`. A changed type rewrites the column's data on the server.
fn clickhouse(table: &str, changes: &[ColumnChange]) -> Result<Vec<String>> {
    let dialect = SqlDialect::ClickHouse;
    let quote = |name: &str| dialect.quote_ident(name);
    let default_of = |column: &ColumnDef| {
        column
            .default_value
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
    };
    let define = |column: &ColumnDef| -> Result<String> {
        checked(&column.type_name, "a column type")?;
        let mut text = format!(
            "{} {}",
            quote(&column.name),
            clickhouse_type(&column.type_name, column.nullable)
        );
        if let Some(default) = default_of(column) {
            text.push(' ');
            text.push_str(&clickhouse_default(&default)?);
        }
        Ok(text)
    };
    let mut out = Vec::new();
    for change in changes {
        if let ColumnChange::Drop { column } = change {
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
                "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                quote(&from.name),
                quote(&to.name)
            ));
        }
        let name = quote(&to.name);
        let before = clickhouse_type(&from.type_name, from.nullable);
        let after = clickhouse_type(&to.type_name, to.nullable);
        let retyped = before != after;
        match (default_of(from) != default_of(to), default_of(to)) {
            // One statement: a type given alone would keep the old default.
            (true, Some(_)) => {
                out.push(format!("ALTER TABLE {table} MODIFY COLUMN {}", define(to)?))
            }
            (true, None) => {
                out.push(format!(
                    "ALTER TABLE {table} MODIFY COLUMN {name} REMOVE DEFAULT"
                ));
                if retyped {
                    checked(&to.type_name, "a column type")?;
                    out.push(format!("ALTER TABLE {table} MODIFY COLUMN {name} {after}"));
                }
            }
            (false, _) if retyped => {
                checked(&to.type_name, "a column type")?;
                out.push(format!("ALTER TABLE {table} MODIFY COLUMN {name} {after}"));
            }
            (false, _) => {}
        }
    }
    for change in changes {
        if let ColumnChange::Add { column } = change {
            out.push(format!(
                "ALTER TABLE {table} ADD COLUMN {}",
                define(column)?
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
    let atomic = conn.transactional_ddl();
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
            } else if index == 1 {
                "The statement before it was applied and cannot be rolled back.".to_owned()
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
        statements(dialect, None, table, &set(changes).changes).unwrap()
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
    fn indexes_keys_and_triggers_per_dialect() {
        let index = ColumnChange::AddIndex {
            name: "by_user".into(),
            columns: vec![
                IndexColumn {
                    name: "user_id".into(),
                    descending: false,
                },
                IndexColumn {
                    name: "created_at".into(),
                    descending: true,
                },
            ],
            unique: true,
        };
        let key = ColumnChange::AddForeignKey {
            name: "orders_user".into(),
            columns: vec!["user_id".into()],
            ref_namespace: None,
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            on_delete: Some("set  null".into()),
            on_update: None,
        };
        let drops = [
            ColumnChange::DropIndex {
                name: "old_idx".into(),
            },
            ColumnChange::DropForeignKey {
                name: "old_fk".into(),
            },
            ColumnChange::DropTrigger {
                name: "audit".into(),
            },
        ];
        let all = |dialect, ns: Option<&str>, table: &str| {
            let mut changes = vec![key.clone(), index.clone()];
            changes.extend(drops.iter().cloned());
            changes.push(ColumnChange::Add {
                column: column("qty", "int", true, None),
            });
            statements(dialect, ns, table, &changes).unwrap()
        };
        assert_eq!(
            all(SqlDialect::Postgres, Some("shop"), "\"shop\".\"orders\""),
            [
                "DROP INDEX \"shop\".\"old_idx\"",
                "ALTER TABLE \"shop\".\"orders\" DROP CONSTRAINT \"old_fk\"",
                "DROP TRIGGER \"audit\" ON \"shop\".\"orders\"",
                "ALTER TABLE \"shop\".\"orders\" ADD COLUMN \"qty\" int",
                "CREATE UNIQUE INDEX \"by_user\" ON \"shop\".\"orders\" (\"user_id\", \"created_at\" DESC)",
                "ALTER TABLE \"shop\".\"orders\" ADD CONSTRAINT \"orders_user\" FOREIGN KEY (\"user_id\") REFERENCES \"shop\".\"users\" (\"id\") ON DELETE SET NULL",
            ]
        );
        let mysql = all(SqlDialect::MySql, Some("shop"), "`shop`.`orders`");
        assert_eq!(mysql[0], "DROP INDEX `old_idx` ON `shop`.`orders`");
        assert_eq!(
            mysql[1],
            "ALTER TABLE `shop`.`orders` DROP FOREIGN KEY `old_fk`"
        );
        assert_eq!(mysql[2], "DROP TRIGGER `shop`.`audit`");
        let tsql = all(SqlDialect::SqlServer, Some("dbo"), "[dbo].[orders]");
        assert_eq!(tsql[0], "DROP INDEX [old_idx] ON [dbo].[orders]");
        assert_eq!(tsql[2], "DROP TRIGGER [dbo].[audit]");
        assert_eq!(tsql[3], "ALTER TABLE [dbo].[orders] ADD [qty] int");

        // SQLite keeps foreign keys in the table's definition.
        let sqlite = statements(
            SqlDialect::Sqlite,
            None,
            "\"orders\"",
            &[index.clone(), drops[0].clone(), drops[2].clone()],
        )
        .unwrap();
        assert_eq!(
            sqlite,
            [
                "DROP INDEX \"old_idx\"",
                "DROP TRIGGER \"audit\"",
                "CREATE UNIQUE INDEX \"by_user\" ON \"orders\" (\"user_id\", \"created_at\" DESC)"
            ]
        );
        assert!(matches!(
            statements(SqlDialect::Sqlite, None, "t", std::slice::from_ref(&key)),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            statements(
                SqlDialect::ClickHouse,
                None,
                "t",
                std::slice::from_ref(&index)
            ),
            Err(Error::Unsupported(_))
        ));

        // Nonsense is refused before anything runs.
        let bad =
            |change: ColumnChange| statements(SqlDialect::Postgres, None, "t", &[change]).is_err();
        assert!(bad(ColumnChange::AddIndex {
            name: " ".into(),
            columns: vec![IndexColumn {
                name: "a".into(),
                descending: false
            }],
            unique: false
        }));
        assert!(bad(ColumnChange::AddIndex {
            name: "i".into(),
            columns: vec![],
            unique: false
        }));
        assert!(bad(ColumnChange::AddForeignKey {
            name: "f".into(),
            columns: vec!["a".into(), "b".into()],
            ref_namespace: None,
            ref_table: "u".into(),
            ref_columns: vec!["id".into()],
            on_delete: None,
            on_update: None,
        }));
        assert!(bad(ColumnChange::AddForeignKey {
            name: "f".into(),
            columns: vec!["a".into()],
            ref_namespace: None,
            ref_table: "u".into(),
            ref_columns: vec!["id".into()],
            on_delete: Some("DROP TABLE x".into()),
            on_update: None,
        }));
    }

    #[test]
    fn a_trigger_is_replaced_by_dropping_it_first() {
        let body = "CREATE TRIGGER audit AFTER UPDATE ON orders FOR EACH ROW BEGIN\n  INSERT INTO log VALUES (NEW.id);\nEND;";
        let out = statements(
            SqlDialect::Sqlite,
            None,
            "\"orders\"",
            &[
                ColumnChange::CreateTrigger {
                    name: "audit".into(),
                    definition: format!("  {body}  "),
                },
                ColumnChange::Add {
                    column: column("note", "TEXT", true, None),
                },
                ColumnChange::DropTrigger {
                    name: "audit".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            [
                "DROP TRIGGER \"audit\"",
                "ALTER TABLE \"orders\" ADD COLUMN \"note\" TEXT",
                // The body keeps its own semicolons; only the closing one goes.
                "CREATE TRIGGER audit AFTER UPDATE ON orders FOR EACH ROW BEGIN\n  INSERT INTO log VALUES (NEW.id);\nEND",
            ]
        );
        let empty = ColumnChange::CreateTrigger {
            name: "t".into(),
            definition: " ; ".into(),
        };
        assert!(statements(SqlDialect::Postgres, None, "t", &[empty]).is_err());
        let duck = ColumnChange::CreateTrigger {
            name: "t".into(),
            definition: "CREATE TRIGGER t".into(),
        };
        assert!(matches!(
            statements(SqlDialect::DuckDb, None, "t", &[duck]),
            Err(Error::Unsupported(_))
        ));
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
    fn clickhouse_keeps_null_in_the_type() {
        assert_eq!(clickhouse_type("String", true), "Nullable(String)");
        assert_eq!(clickhouse_type("Nullable(String)", false), "String");
        assert_eq!(
            clickhouse_type("Nullable(String)", true),
            "Nullable(String)"
        );
        assert_eq!(
            clickhouse_type("LowCardinality(String)", true),
            "LowCardinality(Nullable(String))"
        );
        assert_eq!(
            clickhouse_type("LowCardinality(Nullable(String))", false),
            "LowCardinality(String)"
        );
        let table = "`shop`.`orders`";
        let out = sql(
            SqlDialect::ClickHouse,
            table,
            vec![
                ColumnChange::Add {
                    column: column("qty", "UInt32", false, Some("1")),
                },
                ColumnChange::Modify {
                    from: column("note", "Nullable(String)", true, None),
                    to: column("remark", "Nullable(String)", false, None),
                },
                ColumnChange::Modify {
                    from: column("total", "Float64", false, Some("0")),
                    to: column("total", "Decimal(18, 2)", false, None),
                },
                ColumnChange::Modify {
                    from: column("city", "String", false, None),
                    to: column("city", "String", false, Some("MATERIALIZED lower(name)")),
                },
                ColumnChange::Drop {
                    column: "old".into(),
                },
            ],
        );
        assert_eq!(
            out,
            [
                "ALTER TABLE `shop`.`orders` DROP COLUMN `old`",
                "ALTER TABLE `shop`.`orders` RENAME COLUMN `note` TO `remark`",
                "ALTER TABLE `shop`.`orders` MODIFY COLUMN `remark` String",
                "ALTER TABLE `shop`.`orders` MODIFY COLUMN `total` REMOVE DEFAULT",
                "ALTER TABLE `shop`.`orders` MODIFY COLUMN `total` Decimal(18, 2)",
                "ALTER TABLE `shop`.`orders` MODIFY COLUMN `city` String MATERIALIZED lower(name)",
                "ALTER TABLE `shop`.`orders` ADD COLUMN `qty` UInt32 DEFAULT 1",
            ]
        );
        assert!(!transactional(SqlDialect::ClickHouse));
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
            statements(
                SqlDialect::Postgres,
                None,
                "t",
                std::slice::from_ref(&renamed)
            )
            .unwrap(),
            [
                "ALTER TABLE t RENAME COLUMN \"id\" TO \"key\"",
                "ALTER TABLE t ALTER COLUMN \"key\" TYPE bigint",
                "ALTER TABLE t ALTER COLUMN \"key\" SET DEFAULT 0",
                "ALTER TABLE t ALTER COLUMN \"key\" SET NOT NULL",
            ]
        );
        assert_eq!(
            statements(SqlDialect::MySql, None, "t", &[renamed]).unwrap(),
            ["ALTER TABLE t CHANGE COLUMN `id` `key` bigint DEFAULT 0 NOT NULL"]
        );
    }
}
