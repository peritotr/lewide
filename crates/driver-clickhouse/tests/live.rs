//! Tests against a real server. They run only when `DBCLIENT_TEST_CLICKHOUSE_URL`
//! is set, for example `http://lewide:secret@127.0.0.1:8123/dbclient_test`
//! (see `scripts/test-clickhouse.sh`). Each test works in a database of its own,
//! so they can run at the same time.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dbclient_core::edit::Cell;
use dbclient_core::sql::literal;
use dbclient_core::{
    ChangeSet, Connection, ConnectionConfig, Driver, Error, ObjectKind, QueryRequest, RowChange,
    SortColumn, SqlDialect, TableRequest, Value,
};
use dbclient_driver_clickhouse::ClickHouseDriver;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_CLICKHOUSE_URL").ok()?;
    let rest = url.strip_prefix("http://").expect("an http:// URL");
    let (credentials, rest) = rest.rsplit_once('@').expect("user and password in the URL");
    let (user, password) = credentials.split_once(':').expect("user:password");
    let (address, database) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = address.rsplit_once(':').expect("host:port");
    Some(ConnectionConfig {
        driver: "clickhouse".into(),
        host: Some(host.into()),
        port: Some(port.parse().expect("a port number")),
        user: Some(user.into()),
        password: Some(password.into()),
        database: Some(database.into()).filter(|d: &String| !d.is_empty()),
        ..ConnectionConfig::default()
    })
}

macro_rules! connect_or_skip {
    () => {
        match test_config() {
            Some(config) => ClickHouseDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_CLICKHOUSE_URL is not set");
                return;
            }
        }
    };
}

fn query(statement: &str, max_rows: Option<usize>) -> QueryRequest {
    QueryRequest {
        statement: statement.into(),
        max_rows,
    }
}

async fn fresh_database(conn: &dyn Connection, name: &str) {
    conn.execute_all(query(
        &format!("DROP DATABASE IF EXISTS {name}; CREATE DATABASE {name}"),
        None,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn reads_typed_values() {
    let conn = connect_or_skip!();
    let result = conn
        .execute(query(
            "SELECT NULL, toInt32(-42), toUInt64(18446744073709551615), toInt64(-9223372036854775808), \
               toFloat64(1.5), toDecimal64(12.345, 3), 'héllo ğ \\\\ \\' x', toDate('2026-01-02'), \
               toDateTime64('2026-01-02 10:20:30.123', 3, 'UTC'), \
               toUUID('6f9619ff-8b86-d011-b42d-00c04fd430c8'), true, [1, 2], map('a', 1), \
               CAST('b' AS Enum8('a' = 1, 'b' = 2)), toLowCardinality('lc'), CAST(NULL AS Nullable(Int8))",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            Value::Null,
            Value::Int(-42),
            Value::Decimal("18446744073709551615".into()),
            Value::Int(i64::MIN),
            Value::Float(1.5),
            Value::Decimal("12.345".into()),
            Value::Text("héllo ğ \\ ' x".into()),
            Value::Temporal("2026-01-02".into()),
            Value::Temporal("2026-01-02 10:20:30.123".into()),
            Value::Text("6f9619ff-8b86-d011-b42d-00c04fd430c8".into()),
            Value::Bool(true),
            Value::Json(serde_json::json!([1, 2])),
            Value::Json(serde_json::json!({"a": 1})),
            Value::Text("b".into()),
            Value::Text("lc".into()),
            Value::Null,
        ]
    );
    assert_eq!(result.columns[2].type_name.as_deref(), Some("UInt64"));
}

#[tokio::test]
async fn runs_statements_counts_writes_and_reports_errors() {
    let conn = connect_or_skip!();
    fresh_database(conn.as_ref(), "t_run").await;
    let results = conn
        .execute_all(query(
            "CREATE TABLE t_run.n (id UInt32, label String) ENGINE = MergeTree ORDER BY id;\n\
             -- a comment; with a semicolon\n\
             INSERT INTO t_run.n SELECT number, toString(number) FROM numbers(5);\n\
             SELECT count() AS c FROM t_run.n;",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[1].rows_affected, Some(5));
    assert_eq!(results[2].rows[0][0], Value::Int(5));

    let truncated = conn
        .execute(query("SELECT number FROM numbers(100)", Some(10)))
        .await
        .unwrap();
    assert_eq!(truncated.rows.len(), 10);
    assert!(truncated.truncated);

    // USE is remembered between requests, which HTTP itself does not do.
    conn.execute(query("USE t_run", None)).await.unwrap();
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("t_run")
    );
    let unqualified = conn
        .execute(query("SELECT max(id) FROM n", None))
        .await
        .unwrap();
    assert_eq!(unqualified.rows[0][0], Value::Int(4));
    assert!(conn.execute(query("USE no_such_db", None)).await.is_err());
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("t_run")
    );

    // A FORMAT of the statement's own comes back as lines of text.
    let csv = conn
        .execute(query("SELECT 1, 'a' FORMAT CSV", None))
        .await
        .unwrap();
    assert_eq!(csv.rows, [vec![Value::Text("1,\"a\"".into())]]);

    // The position counts from the top of the editor text, in characters.
    let error = conn
        .execute_all(query("SELECT 'ğ';\nSELEC 1", None))
        .await
        .unwrap_err();
    match error {
        Error::Database { code, position, .. } => {
            assert_eq!(code.as_deref(), Some("62 (SYNTAX_ERROR)"));
            assert_eq!(position, Some(13));
        }
        other => panic!("{other:?}"),
    }
    match conn.execute(query("SELECT * FROM t_run.nope", None)).await {
        Err(Error::Database { code, message, .. }) => {
            assert!(code.unwrap().contains("UNKNOWN_TABLE"));
            assert!(message.contains("nope"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(conn.is_alive().await);
}

#[tokio::test]
async fn lists_and_describes_objects() {
    let conn = connect_or_skip!();
    fresh_database(conn.as_ref(), "t_cat").await;
    conn.execute_all(query(
        "CREATE TABLE t_cat.events (
           id UInt64,
           ts DateTime,
           name LowCardinality(String) COMMENT 'Shown in reports',
           amount Nullable(Decimal(10, 2)),
           day Date MATERIALIZED toDate(ts),
           label String DEFAULT 'none',
           doubled UInt64 ALIAS id * 2,
           INDEX ix_name name TYPE set(100) GRANULARITY 4
         ) ENGINE = MergeTree PARTITION BY toYYYYMM(ts) ORDER BY (id, toDate(ts));
         INSERT INTO t_cat.events (id, ts, name, amount) VALUES (1, '2026-01-02 10:00:00', 'a', 1.5), (2, '2026-02-03 11:00:00', 'b', NULL);
         CREATE VIEW t_cat.named AS SELECT id, name FROM t_cat.events",
        None,
    ))
    .await
    .unwrap();

    let databases: Vec<_> = conn
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(databases.contains(&"t_cat".to_owned()));
    assert!(!databases.iter().any(|d| d == "system"));

    let objects: Vec<_> = conn
        .list_objects(Some("t_cat"))
        .await
        .unwrap()
        .into_iter()
        .map(|o| (o.name, o.kind, o.row_estimate))
        .collect();
    assert_eq!(
        objects,
        [
            ("events".into(), ObjectKind::Table, Some(2)),
            ("named".into(), ObjectKind::View, None),
        ]
    );

    let structure = conn.table_structure(Some("t_cat"), "events").await.unwrap();
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
            ("id", "UInt64", false, None, false),
            ("ts", "DateTime", false, None, false),
            ("name", "LowCardinality(String)", false, None, false),
            ("amount", "Nullable(Decimal(10, 2))", true, None, false),
            ("day", "Date", false, Some("MATERIALIZED toDate(ts)"), false),
            ("label", "String", false, Some("'none'"), false),
            ("doubled", "UInt64", false, Some("ALIAS id * 2"), false),
        ]
    );
    assert_eq!(
        structure.columns[2].comment.as_deref(),
        Some("Shown in reports")
    );
    let indexes: Vec<_> = structure
        .indexes
        .iter()
        .map(|i| (i.name.as_str(), i.columns.clone(), i.method.as_deref()))
        .collect();
    assert_eq!(
        indexes,
        [
            (
                "ORDER BY",
                vec!["id".to_owned(), "toDate(ts)".to_owned()],
                Some("MergeTree")
            ),
            ("PARTITION BY", vec!["toYYYYMM(ts)".to_owned()], None),
            ("ix_name", vec!["name".to_owned()], Some("set(100)")),
        ]
    );
    assert!(structure.foreign_keys.is_empty() && structure.triggers.is_empty());
    let ddl = structure.ddl.unwrap();
    assert!(ddl.starts_with("CREATE TABLE t_cat.events"), "{ddl}");
    assert!(ddl.contains("ORDER BY (id, toDate(ts))"), "{ddl}");

    assert!(
        conn.describe_object(Some("t_cat"), "missing")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pages_in_key_order_filters_and_counts() {
    let conn = connect_or_skip!();
    fresh_database(conn.as_ref(), "t_page").await;
    // Inserted in several parts, so an unordered read could come back in any order.
    conn.execute_all(query(
        "CREATE TABLE t_page.n (id UInt32, label String) ENGINE = MergeTree ORDER BY id;
         INSERT INTO t_page.n SELECT number + 10, concat('n', toString(number + 10)) FROM numbers(15);
         INSERT INTO t_page.n SELECT number, concat('n', toString(number)) FROM numbers(10)",
        None,
    ))
    .await
    .unwrap();
    let page = |offset, order_by: Vec<SortColumn>, filter: Option<&str>| TableRequest {
        namespace: Some("t_page".into()),
        table: "n".into(),
        offset,
        limit: 10,
        order_by,
        filter: filter.map(str::to_owned),
    };
    let first = conn.fetch_rows(page(0, vec![], None)).await.unwrap();
    let ids: Vec<_> = first.rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(ids, (0..10).map(Value::Int).collect::<Vec<_>>());
    assert!(first.has_more);
    assert!(first.columns.iter().all(|c| !c.primary_key));
    let last = conn.fetch_rows(page(20, vec![], None)).await.unwrap();
    assert_eq!(last.rows.len(), 5);
    assert!(!last.has_more);
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
        conn.count_rows(Some("t_page"), "n", None).await.unwrap(),
        25
    );
    assert_eq!(
        conn.count_rows(Some("t_page"), "n", Some("startsWith(label, 'n1')"))
            .await
            .unwrap(),
        11
    );
}

#[tokio::test]
async fn literals_round_trip_and_grid_edits_are_refused() {
    let conn = connect_or_skip!();
    fresh_database(conn.as_ref(), "t_lit").await;
    conn.execute(query(
        "CREATE TABLE t_lit.t (`we``ird \\\\ name` String, b String) ENGINE = Memory",
        None,
    ))
    .await
    .unwrap();
    let tricky = "O'Brien \\ \\' \"x\"; DROP TABLE t; -- ğ✓";
    let column = SqlDialect::ClickHouse.quote_ident("we`ird \\ name");
    let insert = format!(
        "INSERT INTO t_lit.t ({column}, b) VALUES ({}, {})",
        literal(SqlDialect::ClickHouse, &Value::Text(tricky.into())).unwrap(),
        literal(SqlDialect::ClickHouse, &Value::Bytes(vec![0, 65, 255])).unwrap(),
    );
    conn.execute(query(&insert, None)).await.unwrap();
    let back = conn
        .execute(query(
            &format!("SELECT {column}, hex(b) FROM t_lit.t"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        back.rows[0],
        vec![Value::Text(tricky.into()), Value::Text("0041FF".into())]
    );

    // Changing or deleting a row is refused; new rows (an import) go in one INSERT.
    let set = |changes: Vec<RowChange>| ChangeSet {
        namespace: Some("t_lit".into()),
        table: "t".into(),
        changes,
    };
    let cell = |column: &str, value: &str| Cell {
        column: column.into(),
        value: Value::Text(value.into()),
    };
    let refused = conn
        .apply_changes(&set(vec![RowChange::Delete {
            key: vec![cell("b", "x")],
        }]))
        .await;
    assert!(matches!(refused, Err(Error::Unsupported(_))));
    let added = conn
        .apply_changes(&set(vec![
            RowChange::Insert {
                values: vec![cell("b", "one")],
            },
            RowChange::Insert {
                values: vec![cell("b", "two")],
            },
        ]))
        .await
        .unwrap();
    assert_eq!(added.statements.len(), 1);
    // One bad row and nothing of the INSERT is written: a retry cannot double the others.
    conn.execute(query(
        "CREATE TABLE t_lit.n (x UInt8) ENGINE = MergeTree ORDER BY x",
        None,
    ))
    .await
    .unwrap();
    let mixed = ChangeSet {
        namespace: Some("t_lit".into()),
        table: "n".into(),
        changes: ["1", "2", "not a number"]
            .iter()
            .map(|v| RowChange::Insert {
                values: vec![cell("x", v)],
            })
            .collect(),
    };
    assert!(conn.apply_changes(&mixed).await.is_err());
    let count = conn
        .execute(query("SELECT count() FROM t_lit.n", None))
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], Value::Int(0));
    assert!(!conn.in_transaction().await.unwrap());
}

#[tokio::test]
async fn cancel_stops_a_running_statement() {
    let conn: Arc<dyn Connection> = Arc::from(connect_or_skip!());
    let running = {
        let conn = conn.clone();
        tokio::spawn(async move {
            conn.execute(query(
                "SELECT sleepEachRow(1) FROM numbers(30) SETTINGS max_block_size = 1",
                None,
            ))
            .await
        })
    };
    // Late enough for rows to have arrived: the server then answers 200 with
    // the rows so far, and the cancellation is only in the body.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let started = Instant::now();
    conn.cancel().await.unwrap();
    match running.await.unwrap() {
        Err(Error::Database { code, .. }) => {
            assert_eq!(code.as_deref(), Some("394 (QUERY_WAS_CANCELLED)"))
        }
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(5));
    // Nothing running: cancelling is harmless.
    conn.cancel().await.unwrap();
    assert!(conn.is_alive().await);
}

#[tokio::test]
async fn wrong_password_unknown_database_and_closed_port() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_CLICKHOUSE_URL is not set");
        return;
    };
    let wrong = ConnectionConfig {
        password: Some("not the password".into()),
        ..config.clone()
    };
    match ClickHouseDriver.connect(&wrong).await {
        Err(Error::Connect(message)) => {
            assert!(message.to_lowercase().contains("password"), "{message}")
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("connected with a wrong password"),
    }
    let missing = ConnectionConfig {
        database: Some("no_such_database".into()),
        ..config.clone()
    };
    assert!(matches!(
        ClickHouseDriver.connect(&missing).await,
        Err(Error::Connect(_))
    ));
    let closed = ConnectionConfig {
        port: Some(1),
        ..config.clone()
    };
    assert!(matches!(
        ClickHouseDriver.connect(&closed).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn route_and_close() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_CLICKHOUSE_URL is not set");
        return;
    };
    // Through a tunnel the address differs from the host name.
    let routed = ConnectionConfig {
        host: Some("db.example.invalid".into()),
        route: Some(
            format!("127.0.0.1:{}", config.port.unwrap())
                .parse()
                .unwrap(),
        ),
        ..config.clone()
    };
    let conn = ClickHouseDriver.connect(&routed).await.unwrap();
    assert!(
        conn.server_version()
            .await
            .unwrap()
            .unwrap()
            .starts_with("25.")
    );
    conn.close().await.unwrap();
    assert!(!conn.is_alive().await);
    assert!(matches!(conn.ping().await, Err(Error::ConnectionLost(_))));
}

#[tokio::test]
async fn changes_columns_with_alter_table() {
    use dbclient_core::structure::{ColumnChange, ColumnDef, apply_on, statements};
    let conn = connect_or_skip!();
    fresh_database(conn.as_ref(), "t_alter").await;
    conn.execute_all(query(
        "CREATE TABLE t_alter.items (id UInt64, note Nullable(String), price Float64 DEFAULT 0, old String)
           ENGINE = MergeTree ORDER BY id;
         INSERT INTO t_alter.items (id, note, price, old) VALUES (1, 'ılık', 2.5, 'x'), (2, NULL, 3, 'y')",
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
    let table = conn.qualified(Some("t_alter"), "items");
    let changes = [
        ColumnChange::Drop {
            column: "old".into(),
        },
        ColumnChange::Modify {
            from: def("note", "Nullable(String)", true, None),
            to: def("remark", "Nullable(String)", true, Some("'none'")),
        },
        ColumnChange::Modify {
            from: def("price", "Float64", false, Some("0")),
            to: def("price", "Decimal(18, 2)", false, None),
        },
        ColumnChange::Add {
            column: def("qty", "UInt32", false, Some("1")),
        },
    ];
    let sql = statements(SqlDialect::ClickHouse, Some("t_alter"), &table, &changes).unwrap();
    apply_on(conn.as_ref(), sql).await.unwrap();

    let columns = conn
        .describe_object(Some("t_alter"), "items")
        .await
        .unwrap()
        .columns;
    let shown: Vec<_> = columns
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.type_name.as_deref().unwrap_or_default(),
                c.nullable,
                c.default_value.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("id", "UInt64", false, None),
            ("remark", "Nullable(String)", true, Some("'none'")),
            ("price", "Decimal(18, 2)", false, None),
            ("qty", "UInt32", false, Some("1")),
        ]
    );
    let rows = conn
        .execute(query(
            "SELECT remark, toString(price), qty FROM t_alter.items ORDER BY id",
            None,
        ))
        .await
        .unwrap()
        .rows;
    assert_eq!(
        rows,
        [
            vec![
                Value::Text("ılık".into()),
                Value::Text("2.5".into()),
                Value::Int(1)
            ],
            vec![Value::Null, Value::Text("3".into()), Value::Int(1)],
        ]
    );

    // A failure after the first statement says what was already applied.
    let bad = [
        ColumnChange::Add {
            column: def("extra", "UInt8", false, None),
        },
        ColumnChange::Add {
            column: def("broken", "NoSuchType", false, None),
        },
    ];
    let sql = statements(SqlDialect::ClickHouse, Some("t_alter"), &table, &bad).unwrap();
    let error = apply_on(conn.as_ref(), sql).await.unwrap_err().to_string();
    assert!(error.contains("statement 2 of 2"), "{error}");
    assert!(error.contains("cannot be rolled back"), "{error}");
    conn.execute(query("DROP DATABASE t_alter", None))
        .await
        .unwrap();
}
