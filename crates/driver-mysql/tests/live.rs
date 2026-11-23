//! Tests against a real server. They run only when `DBCLIENT_TEST_MYSQL_URL`
//! is set, for example `mysql://root:test@127.0.0.1:3306/app`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dbclient_core::{
    Connection, ConnectionConfig, DbObject, Driver, Error, ObjectKind, QueryRequest, TlsConfig,
    TlsMode, Value,
};
use dbclient_driver_mysql::MysqlDriver;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_MYSQL_URL").ok()?;
    let opts = mysql_async::Opts::from_url(&url).expect("invalid DBCLIENT_TEST_MYSQL_URL");
    Some(ConnectionConfig {
        driver: "mysql".into(),
        host: Some(opts.ip_or_hostname().to_owned()),
        port: Some(opts.tcp_port()),
        user: opts.user().map(str::to_owned),
        password: opts.pass().map(str::to_owned),
        database: opts.db_name().map(str::to_owned),
        ..ConnectionConfig::default()
    })
}

macro_rules! connect_or_skip {
    () => {
        match test_config() {
            Some(config) => MysqlDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_MYSQL_URL is not set");
                return;
            }
        }
    };
}

/// The suite also runs against MariaDB, which differs in a few engine details.
async fn mariadb(conn: &dyn Connection) -> bool {
    conn.server_version()
        .await
        .unwrap()
        .is_some_and(|v| v.contains("MariaDB"))
}

/// Names and kinds only: row estimates vary from run to run.
fn kinds(objects: Vec<DbObject>) -> Vec<(String, ObjectKind)> {
    objects.into_iter().map(|o| (o.name, o.kind)).collect()
}

fn query(statement: &str, max_rows: Option<usize>) -> QueryRequest {
    QueryRequest {
        statement: statement.into(),
        max_rows,
    }
}

#[tokio::test]
async fn reads_typed_values() {
    let conn = connect_or_skip!();
    // MariaDB has no JSON type (the name is an alias for text), so the JSON column is asked for only on MySQL.
    let maria = mariadb(conn.as_ref()).await;
    let json = if maria {
        ""
    } else {
        ", CAST('{\"a\": [1, 2]}' AS JSON)"
    };
    let result = conn
        .execute(query(
            &format!(
                "SELECT NULL, 42, 18446744073709551615, 1.5E0, 123.456, 'héllo', x'00ff', \
                        DATE '2026-01-02', TRUE{json}"
            ),
            None,
        ))
        .await
        .unwrap();

    let mut expected = vec![
        Value::Null,
        Value::Int(42),
        Value::Decimal("18446744073709551615".into()),
        Value::Float(1.5),
        Value::Decimal("123.456".into()),
        Value::Text("héllo".into()),
        Value::Bytes(vec![0, 255]),
        Value::Temporal("2026-01-02".into()),
        Value::Int(1),
    ];
    if !maria {
        expected.push(Value::Json(serde_json::json!({"a": [1, 2]})));
    }
    assert_eq!(result.rows[0], expected);

    let type_names: Vec<_> = result
        .columns
        .iter()
        .map(|c| c.type_name.as_deref().unwrap())
        .collect();
    // The engines size integer literals differently; the rest is the same.
    assert_eq!(type_names[2], "bigint unsigned");
    assert_eq!(
        type_names[3..8],
        ["double", "decimal", "varchar", "varbinary", "date"]
    );
    if !maria {
        assert_eq!(type_names[9], "json");
    }
}

#[tokio::test]
async fn runs_all_statements_and_returns_the_last_result() {
    let conn = connect_or_skip!();
    let result = conn
        .execute(query(
            "DROP DATABASE IF EXISTS t_multi;
             CREATE DATABASE t_multi;
             CREATE TABLE t_multi.users (id int AUTO_INCREMENT PRIMARY KEY, name text NOT NULL);
             INSERT INTO t_multi.users (name) VALUES ('ada'), ('linus');
             SELECT id, name FROM t_multi.users ORDER BY id;",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert_eq!(
        result.rows[1],
        vec![Value::Int(2), Value::Text("linus".into())]
    );
    assert_eq!(result.columns[1].type_name.as_deref(), Some("text"));
    assert_eq!(result.rows_affected, None);

    let affected = conn
        .execute(query("UPDATE t_multi.users SET name = upper(name)", None))
        .await
        .unwrap();
    assert_eq!(affected.rows_affected, Some(2));
    assert!(affected.columns.is_empty());

    // A statement without rows after one with rows reports the affected count.
    let mixed = conn
        .execute(query(
            "SELECT * FROM t_multi.users; DELETE FROM t_multi.users WHERE id = 1",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(mixed.rows_affected, Some(1));
    assert!(mixed.rows.is_empty());

    conn.execute(query("DROP DATABASE t_multi", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn truncates_without_breaking_a_transaction() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "CREATE TEMPORARY TABLE keep (x int); START TRANSACTION; INSERT INTO keep VALUES (1)",
        None,
    ))
    .await
    .unwrap();

    // 5000 rows: MariaDB has a built-in sequence table, MySQL needs a recursive query with its limit raised.
    let series = if mariadb(conn.as_ref()).await {
        "SELECT seq FROM seq_1_to_5000"
    } else {
        "SET SESSION cte_max_recursion_depth = 10000; \
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000) \
         SELECT i FROM n"
    };
    let limited = conn.execute(query(series, Some(10))).await.unwrap();
    assert_eq!(limited.rows.len(), 10);
    assert!(limited.truncated);

    // The transaction opened above must still be alive: rolling back removes the row.
    let kept = conn
        .execute(query("SELECT x FROM keep", None))
        .await
        .unwrap();
    assert_eq!(kept.rows, vec![vec![Value::Int(1)]]);
    let after = conn
        .execute(query("ROLLBACK; SELECT count(*) FROM keep", None))
        .await
        .unwrap();
    assert_eq!(after.rows, vec![vec![Value::Int(0)]]);

    let exact = conn
        .execute(query("SELECT 1 UNION ALL SELECT 2", Some(2)))
        .await
        .unwrap();
    assert_eq!(exact.rows.len(), 2);
    assert!(!exact.truncated);
}

#[tokio::test]
async fn lists_and_describes_objects() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP DATABASE IF EXISTS t_meta;
         CREATE DATABASE t_meta;
         CREATE TABLE t_meta.orders (
             id bigint PRIMARY KEY,
             total decimal(10, 2) NOT NULL DEFAULT 0,
             note varchar(40)
         );
         CREATE VIEW t_meta.big_orders AS SELECT * FROM t_meta.orders WHERE total > 100;",
        None,
    ))
    .await
    .unwrap();

    assert_eq!(
        conn.current_namespace().await.unwrap(),
        test_config().unwrap().database
    );

    let namespaces = conn.list_namespaces().await.unwrap();
    assert!(namespaces.iter().any(|n| n.name == "t_meta"));
    assert!(namespaces.iter().all(|n| n.name != "information_schema"));

    assert_eq!(
        kinds(conn.list_objects(Some("t_meta")).await.unwrap()),
        vec![
            ("big_orders".to_string(), ObjectKind::View),
            ("orders".to_string(), ObjectKind::Table),
        ]
    );

    let table = conn
        .describe_object(Some("t_meta"), "orders")
        .await
        .unwrap();
    assert_eq!(table.columns.len(), 3);
    assert!(table.columns[0].primary_key);
    // MariaDB adds a display width: bigint(20).
    assert!(
        table.columns[0]
            .type_name
            .as_deref()
            .unwrap()
            .starts_with("bigint")
    );
    assert!(!table.columns[1].primary_key);
    assert!(!table.columns[1].nullable);
    assert_eq!(table.columns[1].type_name.as_deref(), Some("decimal(10,2)"));
    assert_eq!(table.columns[1].default_value.as_deref(), Some("0.00"));
    assert!(table.columns[2].nullable);
    assert_eq!(table.columns[2].type_name.as_deref(), Some("varchar(40)"));

    assert!(
        conn.describe_object(Some("t_meta"), "missing")
            .await
            .is_err()
    );
    conn.execute(query("DROP DATABASE t_meta", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn sql_errors_leave_the_session_usable() {
    let conn = connect_or_skip!();
    let err = conn
        .execute(query("SELECT * FROM no_such_table", None))
        .await
        .unwrap_err();
    match err {
        Error::Database { code, message, .. } => {
            assert!(message.contains("no_such_table"), "{message}");
            assert_eq!(code.as_deref(), Some("1146 (42S02)"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    // A syntax error names its line.
    let syntax = conn
        .execute(query("SELECT 1\nFROM\nWHERE", None))
        .await
        .unwrap_err();
    assert!(
        matches!(syntax, Error::Database { line: Some(3), .. }),
        "{syntax:?}"
    );
    // An error in the middle of several statements must not leave results pending.
    assert!(
        conn.execute(query(
            "SELECT 1; SELECT * FROM no_such_table; SELECT 3",
            None
        ))
        .await
        .is_err()
    );
    let ok = conn.execute(query("SELECT 1", None)).await.unwrap();
    assert_eq!(ok.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn cancel_interrupts_a_running_statement() {
    let conn: Arc<dyn Connection> = Arc::from(connect_or_skip!());
    let started = Instant::now();
    let runner = {
        let conn = conn.clone();
        tokio::spawn(async move { conn.execute(query("SELECT SLEEP(30)", None)).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    conn.cancel().await.unwrap();

    // An interrupted SLEEP returns 1 instead of failing, so only the timing is checked.
    let _ = runner.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));

    let ok = conn.execute(query("SELECT 2", None)).await.unwrap();
    assert_eq!(ok.rows, vec![vec![Value::Int(2)]]);
}

#[tokio::test]
async fn close_makes_the_session_unusable() {
    let conn = connect_or_skip!();
    conn.close().await.unwrap();
    assert!(matches!(
        conn.execute(query("SELECT 1", None)).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn wrong_password_is_a_connect_error() {
    let Some(mut config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_MYSQL_URL is not set");
        return;
    };
    config.password = Some("definitely-wrong".into());
    let err = MysqlDriver.connect(&config).await.err().unwrap();
    assert!(matches!(err, Error::Connect(_)), "{err:?}");
}

/// Whether the session's own connection is encrypted, as the server sees it.
async fn server_sees_tls(conn: &dyn Connection) -> bool {
    let result = conn
        .execute(query("SHOW SESSION STATUS LIKE 'Ssl_cipher'", None))
        .await
        .unwrap();
    matches!(&result.rows[0][1], Value::Text(cipher) if !cipher.is_empty())
}

#[tokio::test]
async fn tls_modes() {
    let (Some(config), Ok(ca_file)) = (test_config(), std::env::var("DBCLIENT_TEST_TLS_CA")) else {
        eprintln!("skipped: DBCLIENT_TEST_MYSQL_URL or DBCLIENT_TEST_TLS_CA is not set");
        return;
    };
    let with_tls = |mode, ca_file: Option<&str>| ConnectionConfig {
        tls: TlsConfig {
            mode,
            ca_file: ca_file.map(str::to_owned),
        },
        ..config.clone()
    };

    let plain = MysqlDriver
        .connect(&with_tls(TlsMode::Off, None))
        .await
        .unwrap();
    assert!(!server_sees_tls(plain.as_ref()).await);

    // The test server's certificate is signed by a private CA: trusted only when that CA is given.
    let verified = MysqlDriver
        .connect(&with_tls(TlsMode::Verify, Some(&ca_file)))
        .await
        .unwrap();
    assert!(server_sees_tls(verified.as_ref()).await);

    let untrusted = MysqlDriver
        .connect(&with_tls(TlsMode::Verify, None))
        .await
        .err()
        .unwrap();
    assert!(matches!(untrusted, Error::Connect(_)), "{untrusted:?}");

    let insecure = MysqlDriver
        .connect(&with_tls(TlsMode::Insecure, None))
        .await
        .unwrap();
    assert!(server_sees_tls(insecure.as_ref()).await);

    // The certificate names 127.0.0.1 and localhost; any other name must be rejected.
    let route = Some(
        format!("127.0.0.1:{}", config.port.unwrap())
            .parse()
            .unwrap(),
    );
    let wrong_name = ConnectionConfig {
        host: Some("wrong.example".into()),
        route,
        ..with_tls(TlsMode::Verify, Some(&ca_file))
    };
    let rejected = MysqlDriver.connect(&wrong_name).await.err().unwrap();
    assert!(matches!(rejected, Error::Connect(_)), "{rejected:?}");

    // Through a route the real name is what gets checked, so a matching name passes.
    let right_name = ConnectionConfig {
        host: Some("mysql".into()),
        route,
        ..with_tls(TlsMode::Verify, Some(&ca_file))
    };
    let routed = MysqlDriver.connect(&right_name).await.unwrap();
    assert!(server_sees_tls(routed.as_ref()).await);

    let missing_ca = MysqlDriver
        .connect(&with_tls(TlsMode::Verify, Some("/no/such/ca.pem")))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(missing_ca, Error::InvalidConfig(_)),
        "{missing_ca:?}"
    );
}

#[tokio::test]
async fn cancel_works_over_tls() {
    let (Some(config), Ok(ca_file)) = (test_config(), std::env::var("DBCLIENT_TEST_TLS_CA")) else {
        eprintln!("skipped: DBCLIENT_TEST_MYSQL_URL or DBCLIENT_TEST_TLS_CA is not set");
        return;
    };
    let config = ConnectionConfig {
        tls: TlsConfig {
            mode: TlsMode::Verify,
            ca_file: Some(ca_file),
        },
        ..config
    };
    let conn: Arc<dyn Connection> = Arc::from(MysqlDriver.connect(&config).await.unwrap());
    let started = Instant::now();
    let runner = {
        let conn = conn.clone();
        tokio::spawn(async move { conn.execute(query("SELECT SLEEP(30)", None)).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    conn.cancel().await.unwrap();
    let _ = runner.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn execute_all_returns_one_result_per_statement() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP DATABASE IF EXISTS t_all; CREATE DATABASE t_all; CREATE TABLE t_all.t (x int);",
        None,
    ))
    .await
    .unwrap();

    let sets = conn
        .execute_all(query(
            "INSERT INTO t_all.t VALUES (1), (2), (3); SELECT x FROM t_all.t ORDER BY x; SELECT count(*) FROM t_all.t",
            Some(2),
        ))
        .await
        .unwrap();
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[0].rows_affected, Some(3));
    assert!(sets[0].columns.is_empty());
    // The row limit applies to each result on its own.
    assert_eq!(sets[1].rows.len(), 2);
    assert!(sets[1].truncated);
    assert_eq!(sets[2].rows.len(), 1);
    assert!(!sets[2].truncated);

    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.chars().next().unwrap().is_ascii_digit(),
        "{version}"
    );
    conn.execute(query("DROP DATABASE t_all", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn pages_through_a_table_and_reports_row_estimates() {
    use dbclient_core::{SortColumn, TableRequest};

    let conn = connect_or_skip!();
    conn.execute(query("DROP DATABASE IF EXISTS t_page; CREATE DATABASE t_page; CREATE TABLE t_page.items (id int PRIMARY KEY, grp varchar(8), note varchar(16))", None)).await.unwrap();
    conn.execute(query("INSERT INTO t_page.items WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 25) SELECT i, IF(i % 2 = 0, 'even', 'odd'), CONCAT('n', i) FROM n", None)).await.unwrap();
    let page = |offset, order_by: Vec<SortColumn>| TableRequest {
        namespace: Some("t_page".into()),
        table: "items".into(),
        offset,
        limit: 10,
        order_by,
        filter: None,
    };

    let first = conn.fetch_rows(page(0, vec![])).await.unwrap();
    assert_eq!(first.rows.len(), 10);
    assert!(first.has_more);
    assert_eq!(first.rows[0][0], Value::Int(1));
    assert!(first.columns[0].primary_key);

    let sorted = vec![SortColumn {
        column: "grp".into(),
        descending: true,
    }];
    let second = conn.fetch_rows(page(10, sorted)).await.unwrap();
    assert_eq!(second.rows[0][0], Value::Int(21));
    assert_eq!(second.rows[3][0], Value::Int(2));

    let last = conn.fetch_rows(page(20, vec![])).await.unwrap();
    assert_eq!(last.rows.len(), 5);
    assert!(!last.has_more);
    assert_eq!(
        conn.count_rows(Some("t_page"), "items", Some("grp = 'even'"))
            .await
            .unwrap(),
        12
    );

    // After the statistics are refreshed, the object list carries the row estimate.
    conn.execute(query("ANALYZE TABLE t_page.items", None))
        .await
        .unwrap();
    let objects = conn.list_objects(Some("t_page")).await.unwrap();
    let items = objects.iter().find(|o| o.name == "items").unwrap();
    assert_eq!(items.row_estimate, Some(25));

    conn.execute(query("DROP DATABASE t_page", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn lists_indexes() {
    let conn = connect_or_skip!();
    conn.execute(query("DROP DATABASE IF EXISTS t_idx; CREATE DATABASE t_idx; CREATE TABLE t_idx.orders (id int PRIMARY KEY, user_id int, email varchar(80), total decimal(10,2)); CREATE INDEX orders_user ON t_idx.orders (user_id, total); CREATE UNIQUE INDEX orders_email ON t_idx.orders (email);", None)).await.unwrap();
    let indexes = conn.list_indexes(Some("t_idx"), "orders").await.unwrap();
    let find = |name: &str| {
        indexes
            .iter()
            .find(|i| i.name == name)
            .unwrap_or_else(|| panic!("{name} missing in {indexes:?}"))
    };

    let by_user = find("orders_user");
    assert_eq!(by_user.columns, ["user_id", "total"]);
    assert!(!by_user.unique && !by_user.primary);
    let by_email = find("orders_email");
    assert!(by_email.unique);
    assert_eq!(by_email.columns, ["email"]);
    let primary = indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!(primary.columns, ["id"]);
    assert!(
        conn.list_indexes(Some("t_idx"), "missing")
            .await
            .unwrap()
            .is_empty()
    );

    conn.execute(query("DROP DATABASE t_idx", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn describes_the_structure_of_a_table() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP DATABASE IF EXISTS t_struct; DROP DATABASE IF EXISTS t_struct_ref;
         CREATE DATABASE t_struct; CREATE DATABASE t_struct_ref;
         CREATE TABLE t_struct_ref.regions (country varchar(8), code varchar(8), PRIMARY KEY (country, code));
         CREATE TABLE t_struct.users (id int PRIMARY KEY, email varchar(80) NOT NULL);
         CREATE TABLE t_struct.orders (
             id int PRIMARY KEY,
             user_id int NOT NULL,
             country varchar(8), region varchar(8),
             total decimal(10,2) DEFAULT 0,
             CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES t_struct.users (id) ON DELETE CASCADE,
             CONSTRAINT orders_region_fk FOREIGN KEY (country, region)
                 REFERENCES t_struct_ref.regions (country, code) ON UPDATE SET NULL
         );
         CREATE TRIGGER t_struct.orders_touch BEFORE UPDATE ON t_struct.orders
             FOR EACH ROW SET NEW.total = NEW.total;
         CREATE VIEW t_struct.big_orders AS SELECT id, total FROM t_struct.orders WHERE total > 100;",
        None,
    ))
    .await
    .unwrap();

    let structure = conn
        .table_structure(Some("t_struct"), "orders")
        .await
        .unwrap();
    let keys = &structure.foreign_keys;
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert_eq!(keys[0].name.as_deref(), Some("orders_region_fk"));
    assert_eq!(keys[0].columns, ["country", "region"]);
    assert_eq!(
        keys[0].referenced_namespace.as_deref(),
        Some("t_struct_ref")
    );
    assert_eq!(keys[0].referenced_table, "regions");
    assert_eq!(keys[0].referenced_columns, ["country", "code"]);
    assert_eq!(keys[0].on_update.as_deref(), Some("SET NULL"));
    assert_eq!(keys[1].name.as_deref(), Some("orders_user_fk"));
    assert_eq!(keys[1].columns, ["user_id"]);
    // Same database as the table: not repeated.
    assert_eq!(keys[1].referenced_namespace, None);
    assert_eq!(keys[1].referenced_columns, ["id"]);
    assert_eq!(keys[1].on_delete.as_deref(), Some("CASCADE"));

    assert_eq!(structure.triggers.len(), 1, "{:?}", structure.triggers);
    let trigger = &structure.triggers[0];
    assert_eq!(trigger.name, "orders_touch");
    assert_eq!(trigger.timing.as_deref(), Some("BEFORE"));
    assert_eq!(trigger.events, ["UPDATE"]);
    assert!(trigger.definition.as_deref().unwrap().contains("NEW.total"));

    let ddl = structure.ddl.unwrap();
    assert!(ddl.starts_with("CREATE TABLE `orders`"), "{ddl}");
    assert!(
        ddl.contains("orders_region_fk") && ddl.ends_with(';'),
        "{ddl}"
    );
    let view = conn
        .object_ddl(Some("t_struct"), "big_orders")
        .await
        .unwrap()
        .unwrap();
    assert!(
        view.contains("VIEW") && view.contains("big_orders"),
        "{view}"
    );
    assert!(conn.object_ddl(Some("t_struct"), "missing").await.is_err());

    conn.execute(query(
        "DROP DATABASE t_struct; DROP DATABASE t_struct_ref",
        None,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn applies_row_changes_as_one_unit() {
    use dbclient_core::edit::Cell;
    use dbclient_core::{ChangeSet, RowChange, TableRequest};

    let conn = connect_or_skip!();
    conn.execute(query("DROP DATABASE IF EXISTS t_edit; CREATE DATABASE t_edit; CREATE TABLE t_edit.people (id int PRIMARY KEY, name varchar(80) NOT NULL, note varchar(80)); CREATE TABLE t_edit.loose (x int); INSERT INTO t_edit.people VALUES (1, 'Ada', NULL), (2, 'Linus', 'x')", None)).await.unwrap();
    let cell = |column: &str, value: Value| Cell {
        column: column.into(),
        value,
    };
    let key = |id: i64| vec![cell("id", Value::Int(id))];
    let set = |changes: Vec<RowChange>| ChangeSet {
        namespace: Some("t_edit".into()),
        table: "people".into(),
        changes,
    };
    let tricky = "O'Brien \\ \"x\"; DROP TABLE people; -- ğ✓";
    let names = |conn: std::sync::Arc<dyn Connection>| async move {
        let page = conn
            .fetch_rows(TableRequest {
                namespace: Some("t_edit".into()),
                table: "people".into(),
                offset: 0,
                limit: 50,
                order_by: vec![],
                filter: None,
            })
            .await
            .unwrap();
        page.rows
            .into_iter()
            .map(|r| (r[0].clone(), r[1].clone(), r[2].clone()))
            .collect::<Vec<_>>()
    };
    let conn: std::sync::Arc<dyn Connection> = std::sync::Arc::from(conn);

    // Insert, update and delete in one go; awkward text arrives exactly as written.
    let changes = set(vec![
        RowChange::Insert {
            values: vec![
                cell("id", Value::Int(3)),
                cell("name", Value::Text(tricky.into())),
                cell("note", Value::Null),
            ],
        },
        RowChange::Update {
            key: key(1),
            set: vec![
                cell("name", Value::Text("Ada L.".into())),
                cell("note", Value::Text("updated".into())),
            ],
        },
        RowChange::Delete { key: key(2) },
    ]);
    let preview = conn.preview_changes(&changes).await.unwrap();
    assert_eq!(preview.len(), 3);
    assert!(preview[1].starts_with("UPDATE ") && preview[2].starts_with("DELETE FROM "));
    let applied = conn.apply_changes(&changes).await.unwrap();
    assert_eq!(applied.statements, preview);
    assert_eq!(
        names(conn.clone()).await,
        vec![
            (
                Value::Int(1),
                Value::Text("Ada L.".into()),
                Value::Text("updated".into())
            ),
            (Value::Int(3), Value::Text(tricky.into()), Value::Null),
        ]
    );

    // Setting a value to what it already is still counts as that one row.
    conn.apply_changes(&set(vec![RowChange::Update {
        key: key(1),
        set: vec![cell("name", Value::Text("Ada L.".into()))],
    }]))
    .await
    .unwrap();

    // A change that hits no row undoes the ones before it.
    let stale = set(vec![
        RowChange::Update {
            key: key(1),
            set: vec![cell("name", Value::Text("should not stay".into()))],
        },
        RowChange::Delete { key: key(99) },
    ]);
    let error = conn.apply_changes(&stale).await.unwrap_err();
    assert!(error.to_string().contains("Nothing was applied"), "{error}");
    assert_eq!(names(conn.clone()).await[0].1, Value::Text("Ada L.".into()));

    // So does one the database rejects.
    let duplicate = set(vec![
        RowChange::Delete { key: key(3) },
        RowChange::Insert {
            values: vec![
                cell("id", Value::Int(1)),
                cell("name", Value::Text("dup".into())),
            ],
        },
    ]);
    assert!(conn.apply_changes(&duplicate).await.is_err());
    assert_eq!(names(conn.clone()).await.len(), 2);

    // Rows are addressed by the whole primary key and by real columns only.
    let by_name = set(vec![RowChange::Delete {
        key: vec![cell("name", Value::Text("Ada L.".into()))],
    }]);
    assert!(conn.apply_changes(&by_name).await.is_err());
    let bad_column = set(vec![RowChange::Update {
        key: key(1),
        set: vec![cell("name = name; --", Value::Int(1))],
    }]);
    assert!(conn.apply_changes(&bad_column).await.is_err());
    let no_key = ChangeSet {
        table: "loose".into(),
        ..set(vec![RowChange::Delete {
            key: vec![cell("x", Value::Int(1))],
        }])
    };
    assert!(
        conn.apply_changes(&no_key)
            .await
            .unwrap_err()
            .to_string()
            .contains("no primary key")
    );
    assert_eq!(names(conn.clone()).await.len(), 2);

    // Changes run on a connection of their own, so a transaction the user has open is not ended by them.
    assert!(!conn.in_transaction().await.unwrap());
    conn.execute(query(
        "START TRANSACTION; INSERT INTO t_edit.loose VALUES (7)",
        None,
    ))
    .await
    .unwrap();
    assert!(conn.in_transaction().await.unwrap());
    conn.apply_changes(&set(vec![RowChange::Update {
        key: key(1),
        set: vec![cell("note", Value::Text("side".into()))],
    }]))
    .await
    .unwrap();
    assert!(conn.in_transaction().await.unwrap());
    conn.execute(query("ROLLBACK", None)).await.unwrap();
    let loose = conn
        .execute(query("SELECT count(*) FROM t_edit.loose", None))
        .await
        .unwrap();
    assert_eq!(loose.rows[0][0], Value::Int(0));
    assert_eq!(names(conn.clone()).await[0].2, Value::Text("side".into()));

    conn.execute(query("DROP DATABASE t_edit", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_killed_session_is_not_alive_but_a_failed_statement_is() {
    let victim = connect_or_skip!();
    let killer = connect_or_skip!();
    let id = victim
        .execute(query("SELECT CONNECTION_ID()", None))
        .await
        .unwrap()
        .rows[0][0]
        .clone();
    let Value::Int(id) = id else {
        panic!("no connection id: {id:?}")
    };

    assert!(
        victim
            .execute(query("SELECT * FROM no_such_table", None))
            .await
            .is_err()
    );
    assert!(victim.is_alive().await);

    killer
        .execute(query(&format!("KILL {id}"), None))
        .await
        .unwrap();
    assert!(victim.execute(query("SELECT 1", None)).await.is_err());
    assert!(!victim.is_alive().await);
    assert!(killer.is_alive().await);
}

#[tokio::test]
async fn alters_columns() {
    use dbclient_core::structure::{ColumnChange, ColumnDef, apply_on, statements};
    let conn = connect_or_skip!();
    conn.execute(query("DROP DATABASE IF EXISTS t_alter; CREATE DATABASE t_alter; CREATE TABLE t_alter.items (id int PRIMARY KEY, note text)", None)).await.unwrap();
    conn.execute(query(
        "INSERT INTO t_alter.items VALUES (1, 'a'), (2, NULL)",
        None,
    ))
    .await
    .unwrap();
    let def =
        |name: &str, type_name: &str, nullable: bool, default_value: Option<&str>| ColumnDef {
            name: name.into(),
            type_name: type_name.into(),
            nullable,
            default_value: default_value.map(str::to_owned),
        };
    let run = |changes: Vec<ColumnChange>| {
        let conn = &conn;
        async move {
            let sql = statements(
                conn.dialect(),
                &conn.qualified(Some("t_alter"), "items"),
                &changes,
            )?;
            apply_on(conn.as_ref(), sql).await
        }
    };

    // Add a column with a default, rename one and widen its type.
    run(vec![
        ColumnChange::Add {
            column: def("qty", "int", false, Some("1")),
        },
        ColumnChange::Modify {
            from: def("note", "text", true, None),
            to: def("remark", "varchar(40)", true, None),
        },
    ])
    .await
    .unwrap();
    let described = conn
        .describe_object(Some("t_alter"), "items")
        .await
        .unwrap();
    let names: Vec<&str> = described.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "remark", "qty"]);
    assert!(!described.columns[2].nullable);
    let rows = conn
        .execute(query("SELECT qty FROM t_alter.items ORDER BY id", None))
        .await
        .unwrap();
    assert_eq!(rows.rows, vec![vec![Value::Int(1)], vec![Value::Int(1)]]);

    // A statement that fails: the server's message names it.
    let error = run(vec![ColumnChange::Modify {
        from: def("remark", "varchar(40)", true, None),
        to: def("remark", "varchar(40)", false, None),
    }])
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("statement 1 of 1 failed"),
        "{error}"
    );

    // Without transactional DDL the statements before the failure stay, and the error says so.
    let error = run(vec![
        ColumnChange::Add {
            column: def("extra", "text", true, None),
        },
        // A column of that name is already there.
        ColumnChange::Add {
            column: def("id", "int", true, None),
        },
    ])
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("cannot be rolled back"),
        "{error}"
    );
    assert_eq!(
        conn.describe_object(Some("t_alter"), "items")
            .await
            .unwrap()
            .columns
            .len(),
        4
    );

    // Dropping.
    run(vec![ColumnChange::Drop {
        column: "qty".into(),
    }])
    .await
    .unwrap();
    assert!(
        !conn
            .describe_object(Some("t_alter"), "items")
            .await
            .unwrap()
            .columns
            .iter()
            .any(|c| c.name == "qty")
    );

    conn.execute(query("DROP DATABASE t_alter", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_comments_enums_index_details_and_referencing_keys() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP DATABASE IF EXISTS t_meta2; CREATE DATABASE t_meta2;
         CREATE TABLE t_meta2.users (id int PRIMARY KEY, email varchar(80));
         CREATE TABLE t_meta2.orders (id int PRIMARY KEY, user_id int,
                                      status enum('new','paid','it''s shipped') NOT NULL DEFAULT 'new',
                                      total decimal(10,2) COMMENT 'In cents',
                                      CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES t_meta2.users (id) ON DELETE CASCADE);",
        None,
    ))
    .await
    .unwrap();

    let orders = conn
        .table_structure(Some("t_meta2"), "orders")
        .await
        .unwrap();
    let column = |name: &str| orders.columns.iter().find(|c| c.name == name).unwrap();
    assert_eq!(column("total").comment.as_deref(), Some("In cents"));
    assert_eq!(column("id").comment, None);
    assert_eq!(
        column("status").enum_values.as_deref(),
        Some(
            &[
                "new".to_owned(),
                "paid".to_owned(),
                "it's shipped".to_owned()
            ][..]
        )
    );
    assert_eq!(column("total").enum_values, None);
    assert_eq!(
        orders
            .indexes
            .iter()
            .find(|i| i.primary)
            .unwrap()
            .method
            .as_deref(),
        Some("btree")
    );

    let users = conn
        .table_structure(Some("t_meta2"), "users")
        .await
        .unwrap();
    assert_eq!(users.referenced_by.len(), 1, "{:?}", users.referenced_by);
    let reference = &users.referenced_by[0];
    assert_eq!(
        (reference.table.as_str(), reference.namespace.as_deref()),
        ("orders", None)
    );
    assert_eq!(reference.columns, ["user_id"]);
    assert_eq!(reference.referenced_columns, ["id"]);
    assert_eq!(reference.on_delete.as_deref(), Some("CASCADE"));

    conn.execute(query("DROP DATABASE t_meta2", None))
        .await
        .unwrap();
}
