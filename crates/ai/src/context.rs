//! What the assistant is told about a database: its structure, as text.
//!
//! Only names, types, keys, indexes and the database's own row estimates go
//! in. No query here reads table contents, so no row data can leave the
//! machine through this module.

use dbclient_core::{Connection, ObjectKind, Result};
use serde::Serialize;

/// Tables beyond this many are listed by name only, to keep the request small.
pub const MAX_DETAILED_TABLES: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SchemaContext {
    /// The description sent to the model.
    pub text: String,
    pub namespace: Option<String>,
    pub tables_total: usize,
    /// Tables described with columns and indexes; the rest appear by name only.
    pub tables_detailed: usize,
}

/// Describes the namespace's tables. Those named in `sql` come first, so a
/// large schema still describes the tables the question is about.
pub async fn schema_context(
    conn: &dyn Connection,
    namespace: Option<&str>,
    sql: Option<&str>,
) -> Result<SchemaContext> {
    let namespace = match namespace {
        Some(namespace) => Some(namespace.to_owned()),
        None => conn.current_namespace().await?,
    };
    let ns = namespace.as_deref();
    let mut objects = conn.list_objects(ns).await?;

    let mentioned = |name: &str| sql.is_some_and(|sql| mentions(sql, name));
    // Stable sort: mentioned tables first, each group in the database's order.
    objects.sort_by_key(|o| !mentioned(&o.name));

    let mut text = String::new();
    let mut detailed = 0;
    let mut names_only = Vec::new();
    for object in &objects {
        if detailed >= MAX_DETAILED_TABLES {
            names_only.push(object.name.as_str());
            continue;
        }
        // An object that cannot be described (dropped meanwhile, no permission) is listed by name.
        let Ok(description) = conn.describe_object(ns, &object.name).await else {
            names_only.push(object.name.as_str());
            continue;
        };
        detailed += 1;

        let kind = if object.kind == ObjectKind::View {
            "VIEW"
        } else {
            "TABLE"
        };
        let qualified = match ns {
            Some(ns) => format!("{ns}.{}", object.name),
            None => object.name.clone(),
        };
        text.push_str(&format!("{kind} {qualified}"));
        if let Some(rows) = object.row_estimate {
            text.push_str(&format!(" (about {rows} rows)"));
        }
        text.push('\n');
        for column in &description.columns {
            text.push_str(&format!(
                "  {} {}",
                column.name,
                column.type_name.as_deref().unwrap_or("unknown")
            ));
            if column.primary_key {
                text.push_str(" PRIMARY KEY");
            }
            if !column.nullable {
                text.push_str(" NOT NULL");
            }
            text.push('\n');
        }
        if object.kind != ObjectKind::View {
            for index in conn
                .list_indexes(ns, &object.name)
                .await
                .unwrap_or_default()
            {
                // The primary key is already marked on its columns.
                if index.primary {
                    continue;
                }
                let unique = if index.unique { "UNIQUE " } else { "" };
                text.push_str(&format!(
                    "  {unique}INDEX {} ({})\n",
                    index.name,
                    index.columns.join(", ")
                ));
            }
        }
        text.push('\n');
    }
    if !names_only.is_empty() {
        text.push_str(&format!(
            "Other tables and views, not described here: {}\n",
            names_only.join(", ")
        ));
    }
    if objects.is_empty() {
        text.push_str("(no tables or views)\n");
    }

    Ok(SchemaContext {
        text,
        namespace,
        tables_total: objects.len(),
        tables_detailed: detailed,
    })
}

/// Whether `sql` contains `name` as a whole word, ignoring case.
fn mentions(sql: &str, name: &str) -> bool {
    let sql = sql.to_lowercase();
    let name = name.to_lowercase();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    sql.match_indices(&name).any(|(at, _)| {
        let before = sql[..at].chars().next_back();
        let after = sql[at + name.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_whole_word_mentions_only() {
        assert!(mentions("SELECT * FROM Orders o", "orders"));
        assert!(mentions("select * from public.\"orders\"", "orders"));
        assert!(!mentions("SELECT * FROM order_items", "order"));
        assert!(!mentions("SELECT * FROM reorders", "orders"));
    }
}
