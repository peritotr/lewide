//! Tests against real DuckDB files. DuckDB is built in, so they always run;
//! each test works in a file of its own in the temporary directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dbclient_core::edit::Cell;
use dbclient_core::structure::{ColumnChange, ColumnDef, StructureChangeSet};
use dbclient_core::{
    ChangeSet, Connection, ConnectionConfig, Driver, Error, ObjectKind, QueryRequest, RowChange,
    SortColumn, TableRequest, Value,
};
use dbclient_driver_duckdb::DuckDbDriver;

/// A fresh file path; the file and DuckDB's write-ahead log are removed first.
fn fresh(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lewide-duckdb-{}-{name}.duckdb",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("duckdb.wal"));
    path
}

async fn open(path: &Path, create: bool) -> Result<Box<dyn Connection>, Error> {
    let mut config = ConnectionConfig {
        driver: "duckdb".into(),
        database: Some(path.to_string_lossy().into_owned()),
        ..ConnectionConfig::default()
    };
    if create {
        config.params.insert("create".into(), "true".into());
    }
    DuckDbDriver.connect(&config).await
}

fn query(statement: &str, max_rows: Option<usize>) -> QueryRequest {
    QueryRequest {
        statement: statement.into(),
        max_rows,
    }
}

#[tokio::test]
async fn opens_only_existing_files_unless_asked_and_reads_types() {
    let path = fresh("types");
    assert!(matches!(open(&path, false).await, Err(Error::Connect(_))));
    let conn = open(&path, true).await.unwrap();
    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "{version}"
    );

    let result = conn
        .execute(query(
            "SELECT NULL, 42::INTEGER, 170141183460469231731687303715884105727::HUGEINT, 1.5::DOUBLE, \
               123.456::DECIMAL(10,3), 'héllo ğ \\ '' x', '\\x00\\xFF'::BLOB, DATE '2026-01-02', \
               TIMESTAMP '2026-01-02 10:20:30.123456', TIMESTAMPTZ '2026-01-02 10:20:30+00', \
               TIME '10:20:30', INTERVAL '1 year 2 months 3 days 4 hours', \
               '6f9619ff-8b86-d011-b42d-00c04fd430c8'::UUID, true, [1, 2], {'a': 1, 'b': 'x'}, \
               MAP {'k': 1}, 18446744073709551615::UBIGINT",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            Value::Null,
            Value::Int(42),
            Value::Decimal("170141183460469231731687303715884105727".into()),
            Value::Float(1.5),
            Value::Decimal("123.456".into()),
            Value::Text("héllo ğ \\ ' x".into()),
            Value::Bytes(vec![0, 255]),
            Value::Temporal("2026-01-02".into()),
            Value::Temporal("2026-01-02 10:20:30.123456".into()),
            Value::Temporal("2026-01-02 10:20:30+00".into()),
            Value::Temporal("10:20:30".into()),
            Value::Text("1 year 2 months 3 days 04:00:00".into()),
            Value::Text("6f9619ff-8b86-d011-b42d-00c04fd430c8".into()),
            Value::Bool(true),
            Value::Json(serde_json::json!([1, 2])),
            Value::Json(serde_json::json!({"a": 1, "b": "x"})),
            Value::Json(serde_json::json!([{"key": "k", "value": 1}])),
            Value::Decimal("18446744073709551615".into()),
        ]
    );
    assert_eq!(result.columns[1].type_name.as_deref(), Some("INTEGER"));
    assert_eq!(
        result.columns[4].type_name.as_deref(),
        Some("DECIMAL(10,3)")
    );
}

#[tokio::test]
async fn runs_statements_counts_rows_and_reports_errors() {
    let path = fresh("run");
    let conn = open(&path, true).await.unwrap();
    let results = conn
        .execute_all(query(
            "CREATE TABLE n (id INTEGER PRIMARY KEY, label VARCHAR);\n\
             -- a comment; with a semicolon\n\
             INSERT INTO n SELECT i, 'n' || i FROM range(5) t(i);\n\
             UPDATE n SET label = 'x' WHERE id > 2;\n\
             SELECT count(*) AS c FROM n;",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(results.len(), 4);
    // CREATE answers with an empty "Count" column; it is a command, not a result.
    assert!(results[0].columns.is_empty(), "{:?}", results[0].columns);
    assert_eq!(results[0].rows_affected, Some(0));
    // A query that names its own column Count is still a result.
    let named = conn
        .execute(query("SELECT 3 AS Count", None))
        .await
        .unwrap();
    assert_eq!(named.rows, [vec![Value::Int(3)]]);
    assert_eq!(results[1].rows_affected, Some(5));
    assert_eq!(results[2].rows_affected, Some(2));
    assert_eq!(results[3].rows[0][0], Value::Int(5));
    // RETURNING rows are a result, not a count.
    let returned = conn
        .execute(query(
            "INSERT INTO n VALUES (9, 'nine') RETURNING id, label",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        returned.rows,
        [vec![Value::Int(9), Value::Text("nine".into())]]
    );

    let truncated = conn
        .execute(query("SELECT * FROM range(100)", Some(10)))
        .await
        .unwrap();
    assert_eq!(truncated.rows.len(), 10);
    assert!(truncated.truncated);

    // The place of a syntax error counts from the top of the editor text, in characters.
    match conn
        .execute_all(query("SELECT 'ğ';\nSELEC 1", None))
        .await
        .unwrap_err()
    {
        Error::Database { code, position, .. } => {
            assert_eq!(code.as_deref(), Some("Parser Error"));
            assert_eq!(position, Some(13));
        }
        other => panic!("{other:?}"),
    }
    match conn
        .execute(query("SELECT * FROM nope", None))
        .await
        .unwrap_err()
    {
        Error::Database { code, message, .. } => {
            assert_eq!(code.as_deref(), Some("Catalog Error"));
            assert!(message.contains("nope"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(conn.is_alive().await);
    // Extensions are not fetched from the internet.
    assert!(
        conn.execute(query(
            "SELECT * FROM read_csv('https://example.invalid/x.csv')",
            None
        ))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn lists_and_describes_objects() {
    let path = fresh("catalog");
    let conn = open(&path, true).await.unwrap();
    conn.execute_all(query(
        "CREATE SCHEMA shop;
         CREATE TABLE shop.teams (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL UNIQUE);
         CREATE TABLE shop.people (
           id INTEGER PRIMARY KEY,
           name VARCHAR NOT NULL DEFAULT 'x',
           price DECIMAL(10,2),
           team INTEGER REFERENCES shop.teams (id)
         );
         COMMENT ON COLUMN shop.people.name IS 'Shown on badges';
         CREATE INDEX ix_price ON shop.people (price);
         INSERT INTO shop.teams VALUES (1, 'core');
         CREATE VIEW shop.named AS SELECT id, name FROM shop.people",
        None,
    ))
    .await
    .unwrap();

    let schemas: Vec<_> = conn
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(schemas.contains(&"main".to_owned()) && schemas.contains(&"shop".to_owned()));
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("main")
    );

    let objects: Vec<_> = conn
        .list_objects(Some("shop"))
        .await
        .unwrap()
        .into_iter()
        .map(|o| (o.name, o.kind, o.row_estimate))
        .collect();
    assert_eq!(
        objects,
        [
            ("named".into(), ObjectKind::View, None),
            ("people".into(), ObjectKind::Table, Some(0)),
            ("teams".into(), ObjectKind::Table, Some(1)),
        ]
    );

    let structure = conn.table_structure(Some("shop"), "people").await.unwrap();
    let columns: Vec<_> = structure
        .columns
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.type_name.as_deref().unwrap(),
                c.nullable,
                c.default_value.as_deref(),
                c.primary_key,
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("id", "INTEGER", false, None, true),
            ("name", "VARCHAR", false, Some("'x'"), false),
            ("price", "DECIMAL(10,2)", true, None, false),
            ("team", "INTEGER", true, None, false),
        ]
    );
    assert_eq!(
        structure.columns[1].comment.as_deref(),
        Some("Shown on badges")
    );
    let indexes: Vec<_> = structure
        .indexes
        .iter()
        .map(|i| (i.columns.clone(), i.unique, i.primary))
        .collect();
    assert!(
        indexes.contains(&(vec!["id".to_owned()], true, true)),
        "{indexes:?}"
    );
    assert!(
        structure.indexes.iter().any(|i| i.name == "ix_price"),
        "{indexes:?}"
    );
    assert_eq!(structure.foreign_keys.len(), 1);
    assert_eq!(structure.foreign_keys[0].columns, ["team"]);
    assert_eq!(structure.foreign_keys[0].referenced_table, "teams");
    assert_eq!(structure.foreign_keys[0].referenced_columns, ["id"]);
    let incoming = conn.list_referencing(Some("shop"), "teams").await.unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].table, "people");
    let ddl = structure.ddl.unwrap();
    assert!(ddl.starts_with("CREATE TABLE shop.people"), "{ddl}");
    assert!(ddl.contains("CREATE INDEX ix_price"), "{ddl}");
    assert!(
        conn.object_ddl(Some("shop"), "named")
            .await
            .unwrap()
            .unwrap()
            .contains("CREATE VIEW")
    );
    assert!(conn.describe_object(Some("shop"), "missing").await.is_err());
}

#[tokio::test]
async fn pages_filters_counts_and_edits_rows_as_one_unit() {
    let path = fresh("edit");
    let conn: Arc<dyn Connection> = Arc::from(open(&path, true).await.unwrap());
    conn.execute_all(query(
        "CREATE TABLE people (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL, note VARCHAR, raw BLOB);
         INSERT INTO people SELECT i, 'p' || i, NULL, NULL FROM range(1, 26) t(i)",
        None,
    ))
    .await
    .unwrap();
    let page = |offset, order_by: Vec<SortColumn>, filter: Option<&str>| TableRequest {
        namespace: Some("main".into()),
        table: "people".into(),
        offset,
        limit: 10,
        order_by,
        filter: filter.map(str::to_owned),
    };
    let first = conn.fetch_rows(page(0, vec![], None)).await.unwrap();
    assert_eq!(first.rows[0][0], Value::Int(1));
    assert!(first.has_more);
    let sorted = conn
        .fetch_rows(page(
            0,
            vec![SortColumn {
                column: "id".into(),
                descending: true,
            }],
            Some("id % 2 = 0"),
        ))
        .await
        .unwrap();
    assert_eq!(sorted.rows[0][0], Value::Int(24));
    assert_eq!(
        conn.count_rows(Some("main"), "people", Some("name LIKE 'p1%'"))
            .await
            .unwrap(),
        11
    );

    let cell = |column: &str, value: Value| Cell {
        column: column.into(),
        value,
    };
    let key = |id: i64| vec![cell("id", Value::Int(id))];
    let set = |changes: Vec<RowChange>| ChangeSet {
        namespace: Some("main".into()),
        table: "people".into(),
        changes,
    };
    let tricky = "O'Brien \\ \"x\"; DROP TABLE people; -- ğ✓";
    conn.apply_changes(&set(vec![
        RowChange::Insert {
            values: vec![
                cell("id", Value::Int(100)),
                cell("name", Value::Text(tricky.into())),
                cell("raw", Value::Bytes(vec![0, 1, 255])),
            ],
        },
        RowChange::Update {
            key: key(1),
            set: vec![cell("note", Value::Text("updated".into()))],
        },
        RowChange::Delete { key: key(2) },
    ]))
    .await
    .unwrap();
    let back = conn
        .execute(query("SELECT name, raw FROM people WHERE id = 100 UNION ALL SELECT note, NULL FROM people WHERE id = 1", None))
        .await
        .unwrap();
    assert_eq!(
        back.rows[0],
        vec![Value::Text(tricky.into()), Value::Bytes(vec![0, 1, 255])]
    );
    assert_eq!(back.rows[1][0], Value::Text("updated".into()));
    assert_eq!(conn.count_rows(None, "people", None).await.unwrap(), 25);

    // A change that hits no row undoes the ones before it.
    let stale = set(vec![
        RowChange::Update {
            key: key(1),
            set: vec![cell("note", Value::Text("should not stay".into()))],
        },
        RowChange::Delete { key: key(999) },
    ]);
    assert!(
        conn.apply_changes(&stale)
            .await
            .unwrap_err()
            .to_string()
            .contains("Nothing was applied")
    );
    let note = conn
        .execute(query("SELECT note FROM people WHERE id = 1", None))
        .await
        .unwrap();
    assert_eq!(note.rows[0][0], Value::Text("updated".into()));
    assert!(!conn.in_transaction().await.unwrap());

    // A transaction the user opened is left alone.
    conn.execute(query("BEGIN TRANSACTION", None))
        .await
        .unwrap();
    assert!(conn.in_transaction().await.unwrap());
    assert!(
        conn.apply_changes(&set(vec![RowChange::Delete { key: key(1) }]))
            .await
            .is_err()
    );
    conn.execute(query("ROLLBACK", None)).await.unwrap();
    assert!(!conn.in_transaction().await.unwrap());
}

#[tokio::test]
async fn alters_columns() {
    let path = fresh("alter");
    let conn = open(&path, true).await.unwrap();
    conn.execute_all(query(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, a VARCHAR, b INTEGER DEFAULT 1); INSERT INTO t VALUES (1, 'x', 2)",
        None,
    ))
    .await
    .unwrap();
    let def = |name: &str, ty: &str, nullable: bool, default: Option<&str>| ColumnDef {
        name: name.into(),
        type_name: ty.into(),
        nullable,
        default_value: default.map(str::to_owned),
    };
    let changes = StructureChangeSet {
        namespace: None,
        table: "t".into(),
        changes: vec![
            ColumnChange::Modify {
                from: def("a", "VARCHAR", true, None),
                to: def("title", "VARCHAR", false, Some("'none'")),
            },
            ColumnChange::Modify {
                from: def("b", "INTEGER", true, Some("1")),
                to: def("b", "BIGINT", true, None),
            },
            ColumnChange::Add {
                column: def("c", "DOUBLE", true, Some("0.5")),
            },
        ],
    };
    let statements = dbclient_core::structure::statements(
        conn.dialect(),
        None,
        &conn.qualified(None, "t"),
        &changes.changes,
    )
    .unwrap();
    dbclient_core::structure::apply_on(conn.as_ref(), statements)
        .await
        .unwrap();
    let columns: Vec<_> = conn
        .describe_object(None, "t")
        .await
        .unwrap()
        .columns
        .into_iter()
        .map(|c| (c.name, c.type_name.unwrap(), c.nullable, c.default_value))
        .collect();
    assert_eq!(
        columns,
        [
            ("id".into(), "INTEGER".into(), false, None),
            (
                "title".into(),
                "VARCHAR".into(),
                false,
                Some("'none'".into())
            ),
            ("b".into(), "BIGINT".into(), true, None),
            ("c".into(), "DOUBLE".into(), true, Some("0.5".into())),
        ]
    );
}

#[tokio::test]
async fn cancel_close_and_file_lock() {
    let path = fresh("cancel");
    let conn: Arc<dyn Connection> = Arc::from(open(&path, true).await.unwrap());
    let running = {
        let conn = conn.clone();
        tokio::spawn(async move {
            conn.execute(query(
                "SELECT count(*) FROM range(10000000000) a, range(10) b",
                None,
            ))
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = Instant::now();
    conn.cancel().await.unwrap();
    assert!(running.await.unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(conn.is_alive().await);

    conn.close().await.unwrap();
    assert!(matches!(conn.ping().await, Err(Error::ConnectionLost(_))));
    // Closed, the file can be opened again.
    let again = open(&path, false).await.unwrap();
    assert!(again.is_alive().await);
}
