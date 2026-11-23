//! Tests against a real server. They run only when `DBCLIENT_TEST_SQLSERVER_URL`
//! is set, for example `sqlserver://sa:secret@127.0.0.1:1433/dbclient_test`
//! (see `scripts/test-sqlserver.sh`). Each test works in a schema of its own,
//! so they can run at the same time.

use dbclient_core::edit::Cell;
use dbclient_core::{
    ChangeSet, Connection, ConnectionConfig, Driver, Error, ObjectKind, QueryRequest, RowChange,
    SortColumn, TableRequest, TlsConfig, TlsMode, Value,
};
use dbclient_driver_sqlserver::SqlServerDriver;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_SQLSERVER_URL").ok()?;
    let rest = url
        .strip_prefix("sqlserver://")
        .expect("DBCLIENT_TEST_SQLSERVER_URL starts with sqlserver://");
    let (credentials, rest) = rest.rsplit_once('@').expect("user and password in the URL");
    let (user, password) = credentials.split_once(':').expect("user:password");
    let (address, database) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = address.rsplit_once(':').expect("host:port");
    Some(ConnectionConfig {
        driver: "sqlserver".into(),
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
            Some(config) => SqlServerDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_SQLSERVER_URL is not set");
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

/// Drops the schema with everything in it and creates it again, empty.
async fn fresh_schema(conn: &dyn Connection, schema: &str) {
    let reset = format!(
        "IF SCHEMA_ID('{schema}') IS NOT NULL BEGIN \
           DECLARE @s nvarchar(max) = N''; \
           SELECT @s += N'DROP VIEW [{schema}].' + QUOTENAME(name) + N';' FROM sys.views WHERE schema_id = SCHEMA_ID('{schema}'); \
           SELECT @s += N'ALTER TABLE [{schema}].' + QUOTENAME(OBJECT_NAME(parent_object_id)) + N' DROP CONSTRAINT ' + QUOTENAME(name) + N';' FROM sys.foreign_keys WHERE schema_id = SCHEMA_ID('{schema}'); \
           SELECT @s += N'DROP TABLE [{schema}].' + QUOTENAME(name) + N';' FROM sys.tables WHERE schema_id = SCHEMA_ID('{schema}'); \
           SELECT @s += N'DROP PROCEDURE [{schema}].' + QUOTENAME(name) + N';' FROM sys.procedures WHERE schema_id = SCHEMA_ID('{schema}'); \
           EXEC (@s); EXEC (N'DROP SCHEMA [{schema}]'); \
         END\nGO\nCREATE SCHEMA [{schema}]"
    );
    conn.execute_all(query(&reset, None)).await.unwrap();
}

#[tokio::test]
async fn reads_typed_values() {
    let conn = connect_or_skip!();
    let result = conn
        .execute(query(
            "SELECT NULL, 42, CAST(9223372036854775807 AS bigint), CAST(1.5 AS float), \
               CAST(123.456 AS decimal(10,3)), N'héllo ğ✓', 0x00ff, CAST('2026-01-02' AS date), \
               CAST(1 AS bit), CAST('2026-01-02T10:20:30.1234567' AS datetime2), \
               CAST('2026-01-02 10:20:30 +03:00' AS datetimeoffset), \
               CAST('6f9619ff-8b86-d011-b42d-00c04fd430c8' AS uniqueidentifier), \
               CAST(12.3456 AS money), CAST('10:20:30' AS time(0)), CAST('<a>1</a>' AS xml), \
               CAST(-0.05 AS decimal(5,2)), CAST(7 AS tinyint)",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            Value::Null,
            Value::Int(42),
            Value::Int(i64::MAX),
            Value::Float(1.5),
            Value::Decimal("123.456".into()),
            Value::Text("héllo ğ✓".into()),
            Value::Bytes(vec![0, 255]),
            Value::Temporal("2026-01-02".into()),
            Value::Bool(true),
            Value::Temporal("2026-01-02 10:20:30.123456700".into()),
            Value::Temporal("2026-01-02 10:20:30+03:00".into()),
            Value::Text("6F9619FF-8B86-D011-B42D-00C04FD430C8".into()),
            Value::Decimal("12.3456".into()),
            Value::Temporal("10:20:30".into()),
            Value::Text("<a>1</a>".into()),
            Value::Decimal("-0.05".into()),
            Value::Int(7),
        ]
    );
    let types: Vec<_> = result
        .columns
        .iter()
        .map(|c| c.type_name.clone().unwrap())
        .collect();
    assert_eq!(types[1], "int");
    assert_eq!(types[5], "nvarchar");
    assert_eq!(types[9], "datetime2");
}

#[tokio::test]
async fn runs_batches_counts_rows_and_reports_errors() {
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "t_run").await;
    conn.execute(query("CREATE TABLE t_run.n (id int PRIMARY KEY)", None))
        .await
        .unwrap();

    let inserted = conn
        .execute(query("INSERT INTO t_run.n VALUES (1), (2), (3)", None))
        .await
        .unwrap();
    assert!(inserted.columns.is_empty());
    assert_eq!(inserted.rows_affected, Some(3));
    let updated = conn
        .execute(query("UPDATE t_run.n SET id = id + 10 WHERE id > 1;", None))
        .await
        .unwrap();
    assert_eq!(updated.rows_affected, Some(2));

    // GO separates batches; each SELECT is a result of its own, the row count is not one.
    let results = conn
        .execute_all(query(
            "SELECT 1 AS a\nGO\nSELECT 2 AS b; SELECT 3 AS c",
            None,
        ))
        .await
        .unwrap();
    let firsts: Vec<_> = results.iter().map(|r| r.rows[0][0].clone()).collect();
    assert_eq!(firsts, [Value::Int(1), Value::Int(2), Value::Int(3)]);
    assert!(
        results
            .iter()
            .all(|r| r.columns[0].name != "__lewide_rowcount")
    );

    // A procedure body is the whole batch: nothing is appended to it.
    conn.execute_all(query(
        "CREATE PROCEDURE t_run.list AS SELECT id FROM t_run.n ORDER BY id\nGO",
        None,
    ))
    .await
    .unwrap();
    let listed = conn.execute(query("EXEC t_run.list", None)).await.unwrap();
    assert_eq!(listed.rows.len(), 3);

    let truncated = conn
        .execute(query("SELECT id FROM t_run.n ORDER BY id", Some(2)))
        .await
        .unwrap();
    assert_eq!(truncated.rows.len(), 2);
    assert!(truncated.truncated);

    // The line counts from the top of the editor text, across GO.
    let error = conn
        .execute_all(query(
            "SELECT 1\nGO\nSELECT 1\nSELECT * FROM t_run.nope",
            None,
        ))
        .await
        .unwrap_err();
    match error {
        Error::Database {
            code,
            line,
            message,
            ..
        } => {
            assert_eq!(code.as_deref(), Some("208"), "{message}");
            assert_eq!(line, Some(4));
        }
        other => panic!("{other:?}"),
    }
    // The session is still usable after an error.
    assert_eq!(
        conn.execute(query("SELECT COUNT(*) FROM t_run.n", None))
            .await
            .unwrap()
            .rows[0][0],
        Value::Int(3)
    );
    assert!(conn.is_alive().await);
}

#[tokio::test]
async fn lists_and_describes_objects() {
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "t_cat").await;
    conn.execute_all(query(
        "CREATE TABLE t_cat.teams (id int PRIMARY KEY, name nvarchar(20) NOT NULL)
         CREATE TABLE t_cat.people (
           id int IDENTITY(1,1) CONSTRAINT pk_people PRIMARY KEY,
           name nvarchar(50) NOT NULL DEFAULT N'x',
           price decimal(10,2) NULL,
           note nvarchar(max) NULL,
           seen datetime2(0) NULL DEFAULT (getdate()),
           team int NULL CONSTRAINT fk_team REFERENCES t_cat.teams (id) ON DELETE SET NULL,
           total AS (price * 2)
         )
         CREATE UNIQUE INDEX ix_name ON t_cat.people (name, price) WHERE note IS NOT NULL
         INSERT INTO t_cat.teams VALUES (1, N'core')
         EXEC sp_addextendedproperty N'MS_Description', N'Shown on badges', N'SCHEMA', N't_cat', N'TABLE', N'people', N'COLUMN', N'name'
         GO
         CREATE VIEW t_cat.named AS SELECT id, name FROM t_cat.people
         GO
         CREATE TRIGGER t_cat.touch ON t_cat.people AFTER INSERT, UPDATE AS SET NOCOUNT ON",
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
    assert!(schemas.contains(&"dbo".to_owned()) && schemas.contains(&"t_cat".to_owned()));
    assert!(!schemas.iter().any(|s| s == "sys" || s.starts_with("db_")));
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("dbo")
    );

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
            ("named".into(), ObjectKind::View, None),
            ("people".into(), ObjectKind::Table, Some(0)),
            ("teams".into(), ObjectKind::Table, Some(1)),
        ]
    );

    let structure = conn.table_structure(Some("t_cat"), "people").await.unwrap();
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
            ("id", "int", false, Some("IDENTITY(1,1)"), true),
            ("name", "nvarchar(50)", false, Some("N'x'"), false),
            ("price", "decimal(10,2)", true, None, false),
            ("note", "nvarchar(max)", true, None, false),
            ("seen", "datetime2(0)", true, Some("getdate()"), false),
            ("team", "int", true, None, false),
            // The server works out the computed column's type itself.
            (
                "total",
                "decimal(12,2)",
                true,
                Some("AS ([price]*(2))"),
                false
            ),
        ]
    );
    assert_eq!(
        structure.columns[1].comment.as_deref(),
        Some("Shown on badges")
    );

    let indexes: Vec<_> = structure
        .indexes
        .iter()
        .map(|i| {
            (
                i.name.as_str(),
                i.columns.clone(),
                i.unique,
                i.primary,
                i.method.as_deref(),
                i.condition.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        indexes,
        [
            (
                "pk_people",
                vec!["id".to_owned()],
                true,
                true,
                Some("clustered"),
                None
            ),
            (
                "ix_name",
                vec!["name".to_owned(), "price".to_owned()],
                true,
                false,
                Some("nonclustered"),
                Some("[note] IS NOT NULL")
            ),
        ]
    );
    assert!(structure.indexes[0].size.is_some());

    assert_eq!(structure.foreign_keys.len(), 1);
    let fk = &structure.foreign_keys[0];
    assert_eq!(fk.name.as_deref(), Some("fk_team"));
    assert_eq!(fk.columns, ["team"]);
    assert_eq!(fk.referenced_namespace.as_deref(), Some("t_cat"));
    assert_eq!(fk.referenced_table, "teams");
    assert_eq!(fk.referenced_columns, ["id"]);
    assert_eq!(fk.on_delete.as_deref(), Some("SET NULL"));
    assert_eq!(fk.on_update.as_deref(), Some("NO ACTION"));

    let incoming = conn.list_referencing(Some("t_cat"), "teams").await.unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(
        (incoming[0].table.as_str(), incoming[0].columns.clone()),
        ("people", vec!["team".to_owned()])
    );

    assert_eq!(structure.triggers.len(), 1);
    let trigger = &structure.triggers[0];
    assert_eq!(trigger.name, "touch");
    assert_eq!(trigger.timing.as_deref(), Some("AFTER"));
    let mut events = trigger.events.clone();
    events.sort();
    assert_eq!(events, ["INSERT", "UPDATE"]);
    assert!(
        trigger
            .definition
            .as_deref()
            .unwrap()
            .contains("CREATE TRIGGER")
    );

    let ddl = structure.ddl.unwrap();
    assert!(
        ddl.starts_with("CREATE TABLE [t_cat].[people] (\n    [id] int IDENTITY(1,1) NOT NULL,"),
        "{ddl}"
    );
    assert!(
        ddl.contains("CONSTRAINT [pk_people] PRIMARY KEY ([id])"),
        "{ddl}"
    );
    assert!(ddl.contains("CREATE UNIQUE INDEX [ix_name] ON [t_cat].[people] ([name], [price]) WHERE [note] IS NOT NULL;"), "{ddl}");
    let view = conn
        .object_ddl(Some("t_cat"), "named")
        .await
        .unwrap()
        .unwrap();
    assert!(view.starts_with("CREATE VIEW t_cat.named"), "{view}");

    assert!(
        conn.describe_object(Some("t_cat"), "missing")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pages_sorts_filters_and_counts() {
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "t_page").await;
    conn.execute(query(
        "CREATE TABLE t_page.n (id int PRIMARY KEY, label nvarchar(10)); \
         INSERT INTO t_page.n SELECT value, CONCAT(N'n', value) FROM GENERATE_SERIES(1, 25)",
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
    assert_eq!(first.rows.len(), 10);
    assert!(first.has_more);
    assert_eq!(first.rows[0][0], Value::Int(1));
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
    assert_eq!(sorted.rows.len(), 10);
    assert!(sorted.has_more);

    assert_eq!(
        conn.count_rows(Some("t_page"), "n", None).await.unwrap(),
        25
    );
    assert_eq!(
        conn.count_rows(Some("t_page"), "n", Some("label LIKE N'n1%'"))
            .await
            .unwrap(),
        11
    );
    // A table without a primary key still pages: the order is then the server's.
    conn.execute(query(
        "CREATE TABLE t_page.loose (x int); INSERT INTO t_page.loose VALUES (1), (2)",
        None,
    ))
    .await
    .unwrap();
    let loose = conn
        .fetch_rows(TableRequest {
            table: "loose".into(),
            ..page(0, vec![], None)
        })
        .await
        .unwrap();
    assert_eq!(loose.rows.len(), 2);
}

#[tokio::test]
async fn applies_row_changes_as_one_unit() {
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "t_edit").await;
    conn.execute(query(
        "CREATE TABLE t_edit.people (id int PRIMARY KEY, name nvarchar(200) NOT NULL, note nvarchar(max), active bit, raw varbinary(10)); \
         CREATE TABLE t_edit.loose (x int); \
         INSERT INTO t_edit.people VALUES (1, N'Ada', NULL, 1, NULL), (2, N'Linus', N'x', 0, NULL)",
        None,
    ))
    .await
    .unwrap();
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
    let tricky = "O'Brien \\ \"x\"; DROP TABLE people; -- ğ✓ [a]]";
    let conn: std::sync::Arc<dyn Connection> = std::sync::Arc::from(conn);
    let rows = |conn: std::sync::Arc<dyn Connection>| async move {
        conn.fetch_rows(TableRequest {
            namespace: Some("t_edit".into()),
            table: "people".into(),
            offset: 0,
            limit: 50,
            order_by: vec![],
            filter: None,
        })
        .await
        .unwrap()
        .rows
    };

    let changes = set(vec![
        RowChange::Insert {
            values: vec![
                cell("id", Value::Int(3)),
                cell("name", Value::Text(tricky.into())),
                cell("active", Value::Bool(false)),
                cell("raw", Value::Bytes(vec![0, 1, 255])),
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
    let applied = conn.apply_changes(&changes).await.unwrap();
    assert_eq!(applied.statements.len(), 3);
    let now = rows(conn.clone()).await;
    assert_eq!(now.len(), 2);
    assert_eq!(now[0][1], Value::Text("Ada L.".into()));
    assert_eq!(now[0][2], Value::Text("updated".into()));
    assert_eq!(now[1][1], Value::Text(tricky.into()));
    assert_eq!(now[1][3], Value::Bool(false));
    assert_eq!(now[1][4], Value::Bytes(vec![0, 1, 255]));

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
    assert_eq!(rows(conn.clone()).await[0][1], Value::Text("Ada L.".into()));
    assert!(!conn.in_transaction().await.unwrap());

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
    assert_eq!(rows(conn.clone()).await.len(), 2);
    assert!(!conn.in_transaction().await.unwrap());

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

    // A transaction the user opened is left alone.
    conn.execute(query("BEGIN TRANSACTION", None))
        .await
        .unwrap();
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
    assert_eq!(rows(conn.clone()).await.len(), 2);
}

#[tokio::test]
async fn wrong_password_is_a_connect_error() {
    let Some(mut config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_SQLSERVER_URL is not set");
        return;
    };
    config.password = Some("not the password".into());
    match SqlServerDriver.connect(&config).await {
        Err(Error::Connect(message)) => assert!(message.contains("Login failed"), "{message}"),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("connected with a wrong password"),
    }
    config.port = Some(1);
    assert!(matches!(
        SqlServerDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn tls_modes() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_SQLSERVER_URL is not set");
        return;
    };
    let encrypted = |conn: Box<dyn Connection>| async move {
        conn.execute(query(
            "SELECT encrypt_option FROM sys.dm_exec_connections WHERE session_id = @@SPID",
            None,
        ))
        .await
        .unwrap()
        .rows[0][0]
            .clone()
    };
    let with = |mode| ConnectionConfig {
        tls: TlsConfig {
            mode,
            ca_file: None,
        },
        ..config.clone()
    };

    let plain = SqlServerDriver.connect(&with(TlsMode::Off)).await.unwrap();
    assert_eq!(encrypted(plain).await, Value::Text("FALSE".into()));
    let insecure = SqlServerDriver
        .connect(&with(TlsMode::Insecure))
        .await
        .unwrap();
    assert_eq!(encrypted(insecure).await, Value::Text("TRUE".into()));
    // The test server's certificate is self-signed: checking it must fail.
    assert!(matches!(
        SqlServerDriver.connect(&with(TlsMode::Verify)).await,
        Err(Error::Connect(_))
    ));
    let missing_ca = ConnectionConfig {
        tls: TlsConfig {
            mode: TlsMode::Verify,
            ca_file: Some("/nonexistent/ca.pem".into()),
        },
        ..config.clone()
    };
    assert!(matches!(
        SqlServerDriver.connect(&missing_ca).await,
        Err(Error::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn route_and_close() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_SQLSERVER_URL is not set");
        return;
    };
    // Through a tunnel the address differs from the host name the login uses.
    let routed = ConnectionConfig {
        host: Some("db.example.invalid".into()),
        route: Some(
            format!("127.0.0.1:{}", config.port.unwrap())
                .parse()
                .unwrap(),
        ),
        ..config.clone()
    };
    let conn = SqlServerDriver.connect(&routed).await.unwrap();
    assert!(
        conn.server_version()
            .await
            .unwrap()
            .unwrap()
            .contains("(SQL Server 20")
    );
    conn.close().await.unwrap();
    assert!(!conn.is_alive().await);
    assert!(matches!(conn.ping().await, Err(Error::ConnectionLost(_))));
}

#[tokio::test]
async fn changes_columns_with_t_sql() {
    use dbclient_core::structure::{ColumnChange, ColumnDef, apply_on, statements};
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "lw_alter").await;
    conn.execute_all(query(
        "CREATE TABLE lw_alter.items (id int PRIMARY KEY, note nvarchar(20) NULL DEFAULT N'x', old int NOT NULL DEFAULT 0);\n\
         INSERT INTO lw_alter.items (id) VALUES (1)",
        None,
    ))
    .await
    .unwrap();
    let column =
        |name: &str, type_name: &str, nullable: bool, default_value: Option<&str>| ColumnDef {
            name: name.into(),
            type_name: type_name.into(),
            nullable,
            default_value: default_value.map(str::to_owned),
        };
    let changes = [
        // A column with a default constraint can be dropped.
        ColumnChange::Drop {
            column: "old".into(),
        },
        // Renamed, widened, made NOT NULL and given another default.
        ColumnChange::Modify {
            from: column("note", "nvarchar(20)", true, Some("N'x'")),
            to: column("remark", "nvarchar(200)", false, Some("N'Çağrı'")),
        },
        ColumnChange::Add {
            column: column("qty", "int", false, Some("5")),
        },
    ];
    let table = conn.qualified(Some("lw_alter"), "items");
    let list = statements(conn.dialect(), &table, &changes).unwrap();
    apply_on(conn.as_ref(), list).await.unwrap();

    let description = conn
        .describe_object(Some("lw_alter"), "items")
        .await
        .unwrap();
    let columns: Vec<(&str, Option<&str>, bool, Option<&str>)> = description
        .columns
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.type_name.as_deref(),
                c.nullable,
                c.default_value.as_deref(),
            )
        })
        .collect();
    assert_eq!(columns[0].0, "id");
    assert_eq!(
        columns[1],
        ("remark", Some("nvarchar(200)"), false, Some("N'Çağrı'"))
    );
    assert_eq!(columns[2], ("qty", Some("int"), false, Some("5")));
    assert_eq!(columns.len(), 3);
    // The old default stays on the row that had it; a new row gets the new one.
    let rows = conn
        .execute(query(
            "INSERT INTO lw_alter.items (id) VALUES (2);\nSELECT remark, qty FROM lw_alter.items ORDER BY id",
            None,
        ))
        .await
        .unwrap()
        .rows;
    assert_eq!(rows[0], [Value::Text("x".into()), Value::Int(5)]);
    assert_eq!(rows[1], [Value::Text("Çağrı".into()), Value::Int(5)]);
    assert!(!conn.in_transaction().await.unwrap());
}

#[tokio::test]
async fn shows_a_plan_without_running_the_statement() {
    let conn = connect_or_skip!();
    fresh_schema(conn.as_ref(), "lw_plan").await;
    conn.execute_all(query(
        "CREATE TABLE lw_plan.t (id int PRIMARY KEY, v int)",
        None,
    ))
    .await
    .unwrap();
    let sets = conn
        .execute_all(query(
            "SET SHOWPLAN_TEXT ON\nGO\nDELETE FROM lw_plan.t WHERE v > 1\nGO\nSET SHOWPLAN_TEXT OFF",
            None,
        ))
        .await
        .unwrap();
    let text: Vec<String> = sets
        .iter()
        .filter(|s| s.columns.first().is_some_and(|c| c.name == "StmtText"))
        .flat_map(|s| s.rows.iter().map(|r| format!("{:?}", r[0])))
        .collect();
    assert!(text.iter().any(|l| l.contains("Delete")), "{text:?}");
    // The plan mode is off again, and nothing was deleted or run.
    let rows = conn.execute(query("SELECT 1 AS one", None)).await.unwrap();
    assert_eq!(rows.rows, [[Value::Int(1)]]);
}

#[tokio::test]
async fn stop_ends_the_running_request_and_keeps_the_connection() {
    let conn = connect_or_skip!();
    let started = std::time::Instant::now();
    let (waited, stopped) = tokio::join!(
        conn.execute(query("WAITFOR DELAY '00:00:30'", None)),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            conn.cancel().await
        }
    );
    stopped.unwrap();
    let Err(Error::Query(message)) = waited else {
        panic!("a stopped request must fail");
    };
    assert!(message.contains("stopped"), "{message}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    // The driver opened a new session: the connection still works.
    let rows = conn.execute(query("SELECT 2 AS two", None)).await.unwrap();
    assert_eq!(rows.rows, [[Value::Int(2)]]);
    // With nothing running, Stop does nothing.
    conn.cancel().await.unwrap();
    assert!(conn.is_alive().await);
}
