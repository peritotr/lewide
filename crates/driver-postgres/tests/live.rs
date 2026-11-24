//! Tests against a real server. They run only when `DBCLIENT_TEST_POSTGRES_URL`
//! is set, for example `postgres://postgres:test@127.0.0.1:5432/postgres`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dbclient_core::{
    Connection, ConnectionConfig, DbObject, Driver, Error, ObjectKind, QueryRequest, TlsConfig,
    TlsMode, Value,
};
use dbclient_driver_postgres::PostgresDriver;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_POSTGRES_URL").ok()?;
    let parsed: tokio_postgres::Config = url.parse().expect("invalid DBCLIENT_TEST_POSTGRES_URL");
    let host = match parsed.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => panic!("the test URL needs a TCP host"),
    };
    Some(ConnectionConfig {
        driver: "postgres".into(),
        host: Some(host),
        port: parsed.get_ports().first().copied(),
        user: parsed.get_user().map(str::to_owned),
        password: parsed
            .get_password()
            .map(|p| String::from_utf8_lossy(p).into_owned()),
        database: parsed.get_dbname().map(str::to_owned),
        ..ConnectionConfig::default()
    })
}

macro_rules! connect_or_skip {
    () => {
        match test_config() {
            Some(config) => PostgresDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_POSTGRES_URL is not set");
                return;
            }
        }
    };
}

/// The suite also runs against CockroachDB, which differs in a few engine details.
async fn cockroach(conn: &dyn Connection) -> bool {
    let version = conn.execute(query("SELECT version()", None)).await.unwrap();
    matches!(&version.rows[0][0], Value::Text(text) if text.starts_with("CockroachDB"))
}

/// And against YugabyteDB, whose version reads "PostgreSQL 15.12-YB-…".
async fn yugabyte(conn: &dyn Connection) -> bool {
    let version = conn.execute(query("SELECT version()", None)).await.unwrap();
    matches!(&version.rows[0][0], Value::Text(text) if text.contains("-YB-"))
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
    let result = conn
        .execute(query(
            "SELECT NULL::int, true, 9223372036854775807::int8, 1.5::float8, 'NaN'::float8, \
                    12345.678901234567890123::numeric, 'héllo'::text, '\\x00ff'::bytea, \
                    '{\"a\": [1, 2]}'::jsonb, DATE '2026-01-02', ARRAY[1, 2, 3], \
                    '6f1e7f0a-0000-4000-8000-000000000001'::uuid",
            None,
        ))
        .await
        .unwrap();

    assert_eq!(
        result.rows[0],
        vec![
            Value::Null,
            Value::Bool(true),
            Value::Int(i64::MAX),
            Value::Float(1.5),
            Value::Text("NaN".into()),
            Value::Decimal("12345.678901234567890123".into()),
            Value::Text("héllo".into()),
            Value::Bytes(vec![0, 255]),
            Value::Json(serde_json::json!({"a": [1, 2]})),
            Value::Temporal("2026-01-02".into()),
            Value::Text("{1,2,3}".into()),
            Value::Text("6f1e7f0a-0000-4000-8000-000000000001".into()),
        ]
    );
    let type_names: Vec<_> = result
        .columns
        .iter()
        .map(|c| c.type_name.as_deref().unwrap())
        .collect();
    // An untyped integer is 32-bit in PostgreSQL and 64-bit in CockroachDB.
    let int = if cockroach(conn.as_ref()).await {
        "int8"
    } else {
        "int4"
    };
    let int_array = format!("_{int}");
    assert_eq!(
        type_names,
        [
            int,
            "bool",
            "int8",
            "float8",
            "float8",
            "numeric",
            "text",
            "bytea",
            "jsonb",
            "date",
            int_array.as_str(),
            "uuid"
        ]
    );
}

#[tokio::test]
async fn runs_all_statements_and_returns_the_last_result() {
    let conn = connect_or_skip!();
    let result = conn
        .execute(query(
            "DROP SCHEMA IF EXISTS t_multi CASCADE;
             CREATE SCHEMA t_multi;
             CREATE TABLE t_multi.users (id serial PRIMARY KEY, name text NOT NULL);
             INSERT INTO t_multi.users (name) VALUES ('ada'), ('linus');
             SELECT id, name FROM t_multi.users ORDER BY id;",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[1][1], Value::Text("linus".into()));
    assert_eq!(result.rows_affected, None);

    let affected = conn
        .execute(query("UPDATE t_multi.users SET name = upper(name)", None))
        .await
        .unwrap();
    assert_eq!(affected.rows_affected, Some(2));
    assert!(affected.columns.is_empty());

    conn.execute(query("DROP SCHEMA t_multi CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn truncates_without_breaking_a_transaction() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_trunc CASCADE; CREATE SCHEMA t_trunc; CREATE TABLE t_trunc.keep (x int)",
        None,
    ))
    .await
    .unwrap();
    conn.execute(query("BEGIN", None)).await.unwrap();
    conn.execute(query("INSERT INTO t_trunc.keep VALUES (1)", None))
        .await
        .unwrap();

    let limited = conn
        .execute(query("SELECT g FROM generate_series(1, 5000) g", Some(10)))
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 10);
    assert!(limited.truncated);

    // The transaction opened above must still be alive and usable.
    let kept = conn
        .execute(query("SELECT x FROM t_trunc.keep", None))
        .await
        .unwrap();
    assert_eq!(kept.rows, vec![vec![Value::Int(1)]]);
    conn.execute(query("ROLLBACK", None)).await.unwrap();
    conn.execute(query("DROP SCHEMA t_trunc CASCADE", None))
        .await
        .unwrap();

    let exact = conn
        .execute(query("SELECT g FROM generate_series(1, 10) g", Some(10)))
        .await
        .unwrap();
    assert_eq!(exact.rows.len(), 10);
    assert!(!exact.truncated);
}

#[tokio::test]
async fn lists_and_describes_objects() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_meta CASCADE;
         CREATE SCHEMA t_meta;
         CREATE TABLE t_meta.orders (
             id bigint PRIMARY KEY,
             total numeric(10, 2) NOT NULL DEFAULT 0,
             note varchar(40)
         );
         CREATE VIEW t_meta.big_orders AS SELECT * FROM t_meta.orders WHERE total > 100;",
        None,
    ))
    .await
    .unwrap();

    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("public")
    );
    let namespaces = conn.list_namespaces().await.unwrap();
    assert!(namespaces.iter().any(|n| n.name == "t_meta"));
    assert!(namespaces.iter().all(|n| !n.name.starts_with("pg_")));

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
    assert_eq!(table.columns[0].type_name.as_deref(), Some("bigint"));
    assert!(!table.columns[1].primary_key);
    assert!(!table.columns[1].nullable);
    assert_eq!(table.columns[1].type_name.as_deref(), Some("numeric(10,2)"));
    assert_eq!(table.columns[1].default_value.as_deref(), Some("0"));
    assert!(table.columns[2].nullable);
    assert_eq!(
        table.columns[2].type_name.as_deref(),
        Some("character varying(40)")
    );

    assert!(
        conn.describe_object(Some("t_meta"), "missing")
            .await
            .is_err()
    );
    conn.execute(query("DROP SCHEMA t_meta CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn sql_errors_leave_the_session_usable() {
    let conn = connect_or_skip!();
    let cockroach = cockroach(conn.as_ref()).await;
    let err = conn
        .execute(query("SELECT * FROM no_such_table", None))
        .await
        .unwrap_err();
    match err {
        Error::Database {
            code,
            message,
            position,
            ..
        } => {
            assert!(message.contains("no_such_table"), "{message}");
            // undefined_table, pointing at the table name.
            assert_eq!(code.as_deref(), Some("42P01"));
            // CockroachDB names the table but not where it stands.
            if !cockroach {
                assert_eq!(position, Some(15));
            }
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let ok = conn.execute(query("SELECT 1", None)).await.unwrap();
    assert_eq!(ok.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn cancel_interrupts_a_running_statement() {
    let conn: Arc<dyn Connection> = Arc::from(connect_or_skip!());
    let started = Instant::now();
    let runner = {
        let conn = conn.clone();
        tokio::spawn(async move { conn.execute(query("SELECT pg_sleep(30)", None)).await })
    };
    // A cancel that arrives before the statement has started is ignored by the
    // server, which happens on a busy one (YugabyteDB in CI), so it is repeated
    // until the statement ends.
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if runner.is_finished() {
            break;
        }
        conn.cancel().await.unwrap();
    }

    let outcome = runner.await.unwrap();
    // query_canceled
    assert!(
        matches!(&outcome, Err(Error::Database { code, .. }) if code.as_deref() == Some("57014")),
        "{outcome:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));

    let ok = conn.execute(query("SELECT 2", None)).await.unwrap();
    assert_eq!(ok.rows, vec![vec![Value::Int(2)]]);
}

#[tokio::test]
async fn wrong_password_is_a_connect_error() {
    let Some(mut config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_POSTGRES_URL is not set");
        return;
    };
    // A CockroachDB test server runs without authentication and accepts anything.
    if cockroach(PostgresDriver.connect(&config).await.unwrap().as_ref()).await {
        eprintln!("skipped: the server does not check passwords");
        return;
    }
    config.password = Some("definitely-wrong".into());
    let err = PostgresDriver.connect(&config).await.err().unwrap();
    assert!(matches!(err, Error::Connect(_)), "{err:?}");
}

/// Whether the session's own connection is encrypted, as the server sees it.
async fn server_sees_tls(conn: &dyn Connection) -> bool {
    let result = conn
        .execute(query(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            None,
        ))
        .await
        .unwrap();
    result.rows[0][0] == Value::Bool(true)
}

#[tokio::test]
async fn tls_modes() {
    let (Some(config), Ok(ca_file)) = (test_config(), std::env::var("DBCLIENT_TEST_TLS_CA")) else {
        eprintln!("skipped: DBCLIENT_TEST_POSTGRES_URL or DBCLIENT_TEST_TLS_CA is not set");
        return;
    };
    let with_tls = |mode, ca_file: Option<&str>| ConnectionConfig {
        tls: TlsConfig {
            mode,
            ca_file: ca_file.map(str::to_owned),
        },
        ..config.clone()
    };

    let plain = PostgresDriver
        .connect(&with_tls(TlsMode::Off, None))
        .await
        .unwrap();
    assert!(!server_sees_tls(plain.as_ref()).await);

    // The test server's certificate is signed by a private CA: trusted only when that CA is given.
    let verified = PostgresDriver
        .connect(&with_tls(TlsMode::Verify, Some(&ca_file)))
        .await
        .unwrap();
    assert!(server_sees_tls(verified.as_ref()).await);

    let untrusted = PostgresDriver
        .connect(&with_tls(TlsMode::Verify, None))
        .await
        .err()
        .unwrap();
    assert!(matches!(untrusted, Error::Connect(_)), "{untrusted:?}");

    let insecure = PostgresDriver
        .connect(&with_tls(TlsMode::Insecure, None))
        .await
        .unwrap();
    assert!(server_sees_tls(insecure.as_ref()).await);

    // The certificate names 127.0.0.1 and localhost; any other name must be rejected.
    let wrong_name = ConnectionConfig {
        host: Some("wrong.example".into()),
        route: Some(
            format!("127.0.0.1:{}", config.port.unwrap())
                .parse()
                .unwrap(),
        ),
        ..with_tls(TlsMode::Verify, Some(&ca_file))
    };
    let rejected = PostgresDriver.connect(&wrong_name).await.err().unwrap();
    assert!(matches!(rejected, Error::Connect(_)), "{rejected:?}");

    // Through a route the real name is what gets checked, so a matching name passes.
    let right_name = ConnectionConfig {
        host: Some("pg".into()),
        route: wrong_name.route,
        ..with_tls(TlsMode::Verify, Some(&ca_file))
    };
    let routed = PostgresDriver.connect(&right_name).await.unwrap();
    assert!(server_sees_tls(routed.as_ref()).await);

    let missing_ca = PostgresDriver
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
        eprintln!("skipped: DBCLIENT_TEST_POSTGRES_URL or DBCLIENT_TEST_TLS_CA is not set");
        return;
    };
    let config = ConnectionConfig {
        tls: TlsConfig {
            mode: TlsMode::Verify,
            ca_file: Some(ca_file),
        },
        ..config
    };
    let conn: Arc<dyn Connection> = Arc::from(PostgresDriver.connect(&config).await.unwrap());
    let started = Instant::now();
    let runner = {
        let conn = conn.clone();
        tokio::spawn(async move { conn.execute(query("SELECT pg_sleep(30)", None)).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    conn.cancel().await.unwrap();
    assert!(runner.await.unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn execute_all_returns_one_result_per_statement() {
    let conn = connect_or_skip!();
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_all CASCADE; CREATE SCHEMA t_all; CREATE TABLE t_all.t (x int);",
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
    conn.execute(query("DROP SCHEMA t_all CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn pages_through_a_table_and_reports_row_estimates() {
    use dbclient_core::{SortColumn, TableRequest};

    let conn = connect_or_skip!();
    conn.execute(query("DROP SCHEMA IF EXISTS t_page CASCADE; CREATE SCHEMA t_page; CREATE TABLE t_page.items (id int PRIMARY KEY, grp text, note text)", None)).await.unwrap();
    conn.execute(query("INSERT INTO t_page.items SELECT i, CASE WHEN i % 2 = 0 THEN 'even' ELSE 'odd' END, 'n' || i FROM generate_series(1, 25) i", None)).await.unwrap();
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
    conn.execute(query("ANALYZE t_page.items", None))
        .await
        .unwrap();
    let objects = conn.list_objects(Some("t_page")).await.unwrap();
    let items = objects.iter().find(|o| o.name == "items").unwrap();
    // CockroachDB does not expose its statistics through pg_class: no estimate there.
    if !cockroach(conn.as_ref()).await {
        assert_eq!(items.row_estimate, Some(25));
    }

    conn.execute(query("DROP SCHEMA t_page CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn lists_indexes() {
    let conn = connect_or_skip!();
    conn.execute(query("DROP SCHEMA IF EXISTS t_idx CASCADE; CREATE SCHEMA t_idx; CREATE TABLE t_idx.orders (id int PRIMARY KEY, user_id int, email text, total numeric); CREATE INDEX orders_user ON t_idx.orders (user_id, total); CREATE UNIQUE INDEX orders_email ON t_idx.orders (lower(email));", None)).await.unwrap();
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
    // CockroachDB wraps the expression in one more pair of brackets.
    assert_eq!(by_email.columns.len(), 1);
    assert!(
        by_email.columns[0].contains("lower(email)"),
        "{:?}",
        by_email.columns
    );
    let primary = indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!(primary.columns, ["id"]);
    assert!(
        conn.list_indexes(Some("t_idx"), "missing")
            .await
            .unwrap()
            .is_empty()
    );

    conn.execute(query("DROP SCHEMA t_idx CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn describes_the_structure_of_a_table() {
    let conn = connect_or_skip!();
    let cockroach = cockroach(conn.as_ref()).await;
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_struct CASCADE; DROP SCHEMA IF EXISTS t_struct_ref CASCADE;
         CREATE SCHEMA t_struct; CREATE SCHEMA t_struct_ref;
         CREATE TABLE t_struct_ref.regions (country text, code text, PRIMARY KEY (country, code));
         CREATE TABLE t_struct.users (id int PRIMARY KEY, email text NOT NULL);
         CREATE TABLE t_struct.orders (
             id int PRIMARY KEY,
             user_id int NOT NULL CONSTRAINT orders_user_fk REFERENCES t_struct.users ON DELETE CASCADE,
             country text, region text,
             total numeric(10,2) DEFAULT 0 CONSTRAINT orders_total_check CHECK (total >= 0),
             \"Mixed Case\" varchar(20),
             CONSTRAINT orders_region_fk FOREIGN KEY (region, country)
                 REFERENCES t_struct_ref.regions (code, country) ON UPDATE SET NULL
         );
         CREATE INDEX orders_user ON t_struct.orders (user_id, total);
         CREATE VIEW t_struct.big_orders AS SELECT id, total FROM t_struct.orders WHERE total > 100;",
        None,
    ))
    .await
    .unwrap();
    if !cockroach {
        conn.execute(query(
            "CREATE FUNCTION t_struct.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
             CREATE TRIGGER orders_touch BEFORE INSERT OR UPDATE ON t_struct.orders
                 FOR EACH ROW EXECUTE FUNCTION t_struct.touch();",
            None,
        ))
        .await
        .unwrap();
    }

    let structure = conn
        .table_structure(Some("t_struct"), "orders")
        .await
        .unwrap();
    let keys = &structure.foreign_keys;
    assert_eq!(keys.len(), 2, "{keys:?}");
    // In the order the key lists its columns, not the table's.
    assert_eq!(keys[0].name.as_deref(), Some("orders_region_fk"));
    assert_eq!(keys[0].columns, ["region", "country"]);
    assert_eq!(
        keys[0].referenced_namespace.as_deref(),
        Some("t_struct_ref")
    );
    assert_eq!(keys[0].referenced_table, "regions");
    assert_eq!(keys[0].referenced_columns, ["code", "country"]);
    assert_eq!(keys[0].on_update.as_deref(), Some("SET NULL"));
    assert_eq!(keys[0].on_delete.as_deref(), Some("NO ACTION"));
    assert_eq!(keys[1].name.as_deref(), Some("orders_user_fk"));
    assert_eq!(keys[1].columns, ["user_id"]);
    // Same schema as the table: not repeated.
    assert_eq!(keys[1].referenced_namespace, None);
    assert_eq!(keys[1].referenced_columns, ["id"]);
    assert_eq!(keys[1].on_delete.as_deref(), Some("CASCADE"));

    let ddl = structure.ddl.clone().unwrap();
    assert!(ddl.contains("CREATE TABLE"), "{ddl}");
    assert!(
        ddl.contains("orders_region_fk") && ddl.contains("\"Mixed Case\""),
        "{ddl}"
    );

    if !cockroach {
        assert_eq!(structure.triggers.len(), 1, "{:?}", structure.triggers);
        let trigger = &structure.triggers[0];
        assert_eq!(trigger.name, "orders_touch");
        assert_eq!(trigger.timing.as_deref(), Some("BEFORE"));
        assert_eq!(trigger.events, ["INSERT", "UPDATE"]);
        assert!(
            trigger
                .definition
                .as_deref()
                .unwrap()
                .starts_with("CREATE TRIGGER orders_touch")
        );

        // The DDL is put together from the catalog, so prove it: drop the
        // table, run the DDL, and the structure is what it was.
        conn.execute(query("DROP TABLE t_struct.orders CASCADE", None))
            .await
            .unwrap();
        conn.execute(query(&ddl, None))
            .await
            .unwrap_or_else(|e| panic!("{e}\n{ddl}"));
        let again = conn
            .table_structure(Some("t_struct"), "orders")
            .await
            .unwrap();
        assert_eq!(again, structure, "{ddl}");
        // The view went with the table.
        conn.execute(query(
            "CREATE VIEW t_struct.big_orders AS SELECT id, total FROM t_struct.orders WHERE total > 100",
            None,
        ))
        .await
        .unwrap();
    }

    let view = conn
        .object_ddl(Some("t_struct"), "big_orders")
        .await
        .unwrap()
        .unwrap();
    assert!(view.starts_with("CREATE VIEW"), "{view}");
    assert!(
        view.contains("100") && view.trim_end().ends_with(';'),
        "{view}"
    );
    assert_eq!(
        conn.object_ddl(Some("t_struct"), "missing")
            .await
            .ok()
            .flatten(),
        None
    );

    conn.execute(query(
        "DROP SCHEMA t_struct CASCADE; DROP SCHEMA t_struct_ref CASCADE",
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
    conn.execute(query("DROP SCHEMA IF EXISTS t_edit CASCADE; CREATE SCHEMA t_edit; CREATE TABLE t_edit.people (id int PRIMARY KEY, name text NOT NULL, note text); CREATE TABLE t_edit.loose (x int); INSERT INTO t_edit.people VALUES (1, 'Ada', NULL), (2, 'Linus', 'x')", None)).await.unwrap();
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

    // A transaction the user opened is left alone: applying is refused, not folded into it.
    assert!(!conn.in_transaction().await.unwrap());
    conn.execute(query("BEGIN", None)).await.unwrap();
    assert!(conn.in_transaction().await.unwrap());
    let refused = conn
        .apply_changes(&set(vec![RowChange::Delete { key: key(1) }]))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("transaction is open"),
        "{refused}"
    );
    conn.execute(query("ROLLBACK", None)).await.unwrap();
    assert!(!conn.in_transaction().await.unwrap());
    assert_eq!(names(conn.clone()).await.len(), 2);

    conn.execute(query("DROP SCHEMA t_edit CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_killed_session_is_not_alive_but_a_failed_statement_is() {
    let victim = connect_or_skip!();
    let killer = connect_or_skip!();
    if cockroach(killer.as_ref()).await {
        eprintln!("skipped: uses pg_terminate_backend, which CockroachDB does not have");
        return;
    }
    // YugabyteDB treats a terminated backend as a crash and restarts every
    // session, which would break the tests running next to this one.
    if yugabyte(killer.as_ref()).await {
        eprintln!("skipped: pg_terminate_backend restarts the whole YugabyteDB server");
        return;
    }
    let pid = victim
        .execute(query("SELECT pg_backend_pid()", None))
        .await
        .unwrap()
        .rows[0][0]
        .clone();
    let Value::Int(pid) = pid else {
        panic!("no pid")
    };

    // A rejected statement, even one that leaves a transaction failed, is not a lost connection.
    victim.execute(query("BEGIN", None)).await.unwrap();
    assert!(
        victim
            .execute(query("SELECT * FROM no_such_table", None))
            .await
            .is_err()
    );
    assert!(victim.is_alive().await);
    victim.execute(query("ROLLBACK", None)).await.unwrap();

    killer
        .execute(query(&format!("SELECT pg_terminate_backend({pid})"), None))
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
    conn.execute(query("DROP SCHEMA IF EXISTS t_alter CASCADE; CREATE SCHEMA t_alter; CREATE TABLE t_alter.items (id integer PRIMARY KEY, note text)", None)).await.unwrap();
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
                Some("t_alter"),
                &conn.qualified(Some("t_alter"), "items"),
                &changes,
            )?;
            apply_on(conn.as_ref(), sql).await
        }
    };

    // Add a column with a default, rename one and widen its type.
    run(vec![
        ColumnChange::Add {
            column: def("qty", "integer", false, Some("1")),
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

    // With transactional DDL nothing of a failed set stays.
    let error = run(vec![
        ColumnChange::Add {
            column: def("extra", "text", true, None),
        },
        ColumnChange::Drop {
            column: "no_such_column".into(),
        },
    ])
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Nothing was changed"), "{error}");
    assert_eq!(
        conn.describe_object(Some("t_alter"), "items")
            .await
            .unwrap()
            .columns
            .len(),
        3
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

    conn.execute(query("DROP SCHEMA t_alter CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_comments_enums_index_details_and_referencing_keys() {
    let conn = connect_or_skip!();
    let cockroach = cockroach(conn.as_ref()).await;
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_meta2 CASCADE; CREATE SCHEMA t_meta2;
         CREATE TYPE t_meta2.order_status AS ENUM ('new', 'paid', 'shipped');
         CREATE TABLE t_meta2.users (id int PRIMARY KEY, email text);
         CREATE TABLE t_meta2.orders (id int PRIMARY KEY, user_id int REFERENCES t_meta2.users ON DELETE CASCADE,
                                      status t_meta2.order_status NOT NULL DEFAULT 'new', total numeric);
         COMMENT ON COLUMN t_meta2.orders.total IS 'In cents';
         CREATE INDEX orders_paid ON t_meta2.orders (user_id) WHERE status = 'paid';",
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
        Some(&["new".to_owned(), "paid".to_owned(), "shipped".to_owned()][..])
    );
    assert_eq!(column("total").enum_values, None);

    let partial = orders
        .indexes
        .iter()
        .find(|i| i.name == "orders_paid")
        .unwrap();
    let condition = partial.condition.as_deref().unwrap();
    assert!(
        condition.contains("status") && condition.contains("paid"),
        "{condition}"
    );
    let primary = orders.indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!(primary.condition, None);
    if !cockroach {
        // YugabyteDB stores its indexes as LSM trees.
        let method = if yugabyte(conn.as_ref()).await {
            "lsm"
        } else {
            "btree"
        };
        assert_eq!(primary.method.as_deref(), Some(method));
        assert!(primary.size.is_some());
    }

    let users = conn
        .table_structure(Some("t_meta2"), "users")
        .await
        .unwrap();
    assert_eq!(users.referenced_by.len(), 1, "{:?}", users.referenced_by);
    let reference = &users.referenced_by[0];
    assert_eq!(reference.table, "orders");
    assert_eq!(reference.namespace, None);
    assert_eq!(reference.columns, ["user_id"]);
    assert_eq!(reference.referenced_columns, ["id"]);
    assert_eq!(reference.on_delete.as_deref(), Some("CASCADE"));
    assert!(orders.referenced_by.is_empty());

    conn.execute(query("DROP SCHEMA t_meta2 CASCADE", None))
        .await
        .unwrap();
}

/// TimescaleDB only: a hypertable is an ordinary table to the client, and the
/// extension's own schemas (where the chunks live) stay out of the list.
#[tokio::test]
async fn timescaledb_hypertables_read_like_tables() {
    use dbclient_core::TableRequest;
    let conn = connect_or_skip!();
    let installed = conn
        .execute(query(
            "SELECT count(*) FROM pg_extension WHERE extname = 'timescaledb'",
            None,
        ))
        .await
        .unwrap();
    if installed.rows[0][0] != Value::Int(1) {
        eprintln!("skipped: the server is not TimescaleDB");
        return;
    }
    conn.execute(query(
        "DROP SCHEMA IF EXISTS t_ts CASCADE; CREATE SCHEMA t_ts;
         CREATE TABLE t_ts.metrics (at timestamptz NOT NULL, device int NOT NULL, value float8,
                                    PRIMARY KEY (device, at));
         SELECT create_hypertable('t_ts.metrics', by_range('at', INTERVAL '1 day'));
         INSERT INTO t_ts.metrics
             SELECT TIMESTAMPTZ '2026-01-01' + n * INTERVAL '6 hours', n % 3, n
             FROM generate_series(0, 39) AS n;",
        None,
    ))
    .await
    .unwrap();

    let version = conn.server_version().await.unwrap().unwrap();
    assert!(version.contains("(PostgreSQL "), "{version}");

    let namespaces = conn.list_namespaces().await.unwrap();
    assert!(namespaces.iter().any(|n| n.name == "t_ts"));
    assert!(
        namespaces.iter().all(|n| !n.name.contains("timescaledb")),
        "{namespaces:?}"
    );
    assert_eq!(
        kinds(conn.list_objects(Some("t_ts")).await.unwrap()),
        vec![("metrics".to_string(), ObjectKind::Table)]
    );

    // Rows come back across chunks, in key order, with the key marked.
    let page = conn
        .fetch_rows(TableRequest {
            namespace: Some("t_ts".into()),
            table: "metrics".into(),
            offset: 0,
            limit: 50,
            order_by: vec![],
            filter: Some("value >= 30".into()),
        })
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 10);
    assert!(page.columns[0].primary_key && page.columns[1].primary_key);
    assert_eq!(
        conn.count_rows(Some("t_ts"), "metrics", None)
            .await
            .unwrap(),
        40
    );

    conn.execute(query("DROP SCHEMA t_ts CASCADE", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn adds_and_drops_indexes_keys_and_triggers() {
    use dbclient_core::structure::{ColumnChange, IndexColumn, apply_on, statements};
    let conn = connect_or_skip!();
    conn.execute_all(query(
        "DROP SCHEMA IF EXISTS t_keys CASCADE; CREATE SCHEMA t_keys; \
         CREATE TABLE t_keys.users (id integer PRIMARY KEY); \
         CREATE TABLE t_keys.orders (id integer PRIMARY KEY, user_id integer, created integer); \
         CREATE FUNCTION t_keys.noop() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$; \
         CREATE TRIGGER audit BEFORE INSERT ON t_keys.orders FOR EACH ROW EXECUTE FUNCTION t_keys.noop()",
        None,
    ))
    .await
    .unwrap();
    let run = |changes: Vec<ColumnChange>| {
        let conn = &conn;
        async move {
            let sql = statements(
                conn.dialect(),
                Some("t_keys"),
                &conn.qualified(Some("t_keys"), "orders"),
                &changes,
            )?;
            apply_on(conn.as_ref(), sql).await
        }
    };
    run(vec![
        ColumnChange::AddForeignKey {
            name: "orders_user".into(),
            columns: vec!["user_id".into()],
            ref_namespace: None,
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            on_delete: Some("CASCADE".into()),
            on_update: None,
        },
        ColumnChange::AddIndex {
            name: "orders_by_user".into(),
            columns: vec![
                IndexColumn {
                    name: "user_id".into(),
                    descending: false,
                },
                IndexColumn {
                    name: "created".into(),
                    descending: true,
                },
            ],
            unique: false,
        },
    ])
    .await
    .unwrap();
    let structure = conn
        .table_structure(Some("t_keys"), "orders")
        .await
        .unwrap();
    let index = structure
        .indexes
        .iter()
        .find(|i| i.name == "orders_by_user")
        .expect("the new index");
    assert_eq!(index.columns, ["user_id", "created DESC"]);
    let key = structure
        .foreign_keys
        .iter()
        .find(|k| k.name.as_deref() == Some("orders_user"))
        .expect("the new key");
    assert_eq!(key.columns, ["user_id"]);
    assert_eq!(structure.triggers.len(), 1);

    run(vec![
        ColumnChange::DropForeignKey {
            name: "orders_user".into(),
        },
        ColumnChange::DropIndex {
            name: "orders_by_user".into(),
        },
        ColumnChange::DropTrigger {
            name: "audit".into(),
        },
    ])
    .await
    .unwrap();
    let structure = conn
        .table_structure(Some("t_keys"), "orders")
        .await
        .unwrap();
    assert!(structure.indexes.iter().all(|i| i.name != "orders_by_user"));
    assert!(structure.foreign_keys.is_empty());
    assert!(structure.triggers.is_empty());

    // A failing statement leaves nothing half done: PostgreSQL undoes the batch.
    let failed = run(vec![
        ColumnChange::AddIndex {
            name: "ok_index".into(),
            columns: vec![IndexColumn {
                name: "created".into(),
                descending: false,
            }],
            unique: false,
        },
        ColumnChange::DropIndex {
            name: "no_such_index".into(),
        },
    ])
    .await;
    assert!(failed.is_err());
    let structure = conn
        .table_structure(Some("t_keys"), "orders")
        .await
        .unwrap();
    assert!(structure.indexes.iter().all(|i| i.name != "ok_index"));
}

#[tokio::test]
async fn creates_and_replaces_a_trigger_with_its_function() {
    use dbclient_core::structure::{ColumnChange, apply_on, statements};
    let conn = connect_or_skip!();
    // CockroachDB's triggers differ (and arrived late); the suite checks PostgreSQL's.
    if cockroach(conn.as_ref()).await {
        return;
    }
    conn.execute_all(query(
        "DROP TABLE IF EXISTS trig_items; DROP FUNCTION IF EXISTS trig_items_fn();
         CREATE TABLE trig_items (id int PRIMARY KEY, name text, touched int DEFAULT 0)",
        None,
    ))
    .await
    .unwrap();
    let create = |bump: i32| ColumnChange::CreateTrigger {
        name: "trig_items_touch".into(),
        definition: format!(
            "CREATE OR REPLACE FUNCTION trig_items_fn() RETURNS trigger LANGUAGE plpgsql AS $$\n\
             BEGIN\n  NEW.touched := OLD.touched + {bump};\n  RETURN NEW;\nEND;\n$$;\n\
             CREATE TRIGGER trig_items_touch BEFORE UPDATE ON trig_items\n  FOR EACH ROW EXECUTE FUNCTION trig_items_fn();"
        ),
    };
    let table = conn.qualified(None, "trig_items");
    let run = |changes: Vec<ColumnChange>| {
        let sql = statements(conn.dialect(), None, &table, &changes).unwrap();
        apply_on(conn.as_ref(), sql)
    };
    run(vec![create(1)]).await.unwrap();
    run(vec![
        ColumnChange::DropTrigger {
            name: "trig_items_touch".into(),
        },
        create(10),
    ])
    .await
    .unwrap();
    // Several statements in one text: values come back as text.
    let rows = conn
        .execute_all(query(
            "INSERT INTO trig_items (id, name) VALUES (1, 'a');
             UPDATE trig_items SET name = 'ğ' WHERE id = 1;
             SELECT touched FROM trig_items",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(rows.last().unwrap().rows, [vec![Value::Text("10".into())]]);
    let structure = conn.table_structure(None, "trig_items").await.unwrap();
    assert_eq!(structure.triggers.len(), 1);
    assert_eq!(structure.triggers[0].name, "trig_items_touch");

    // A definition that fails leaves the old trigger in place: it all runs in one transaction.
    let error = run(vec![
        ColumnChange::DropTrigger {
            name: "trig_items_touch".into(),
        },
        ColumnChange::CreateTrigger {
            name: "trig_items_touch".into(),
            definition: "CREATE TRIGGER trig_items_touch BEFORE UPDATE ON trig_items FOR EACH ROW EXECUTE FUNCTION no_such_fn()".into(),
        },
    ])
    .await
    .unwrap_err();
    let structure = conn.table_structure(None, "trig_items").await.unwrap();
    // YugabyteDB commits each DDL statement on its own, so the drop stays
    // and the error says so.
    if yugabyte(conn.as_ref()).await {
        assert!(
            error
                .to_string()
                .contains("The statement before it was applied"),
            "{error}"
        );
        assert_eq!(structure.triggers.len(), 0);
    } else {
        assert!(error.to_string().contains("Nothing was changed"), "{error}");
        assert_eq!(structure.triggers.len(), 1);
    }
    conn.execute_all(query(
        "DROP TABLE trig_items; DROP FUNCTION trig_items_fn()",
        None,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn several_statements_inside_an_open_transaction_keep_it_usable() {
    let conn = connect_or_skip!();
    conn.execute(query("BEGIN", None)).await.unwrap();
    // Preparing this text would fail and abort the transaction; it is not tried.
    let sets = conn
        .execute_all(query("SELECT 1 AS a; SELECT 'ğ' AS b", None))
        .await
        .unwrap();
    assert_eq!(sets.len(), 2);
    let one = conn.execute(query("SELECT 3 AS c", None)).await.unwrap();
    assert_eq!(one.rows, [vec![Value::Int(3)]]);
    conn.execute(query("COMMIT", None)).await.unwrap();
}
