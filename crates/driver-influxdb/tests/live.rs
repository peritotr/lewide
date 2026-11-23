//! Tests against a real server. They run only when `DBCLIENT_TEST_INFLUXDB_URL`
//! is set, for example `http://127.0.0.1:8181`; `scripts/test-influxdb.sh up`
//! starts a server, writes the sample data these tests read (`sensors.home`,
//! `garden.soil`) and prints the variables. `DBCLIENT_TEST_INFLUXDB_TOKEN` is
//! an admin token of that server.

use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, QueryRequest, SortColumn,
    TableRequest, Value,
};
use dbclient_driver_influxdb::InfluxDbDriver;

struct Server {
    host: String,
    port: u16,
    token: String,
}

fn server() -> Option<Server> {
    let url = std::env::var("DBCLIENT_TEST_INFLUXDB_URL").ok()?;
    let rest = url.trim_end_matches('/').trim_start_matches("http://");
    let (host, port) = rest.rsplit_once(':').expect("the test URL has a port");
    Some(Server {
        host: host.to_owned(),
        port: port.parse().expect("a port number"),
        token: std::env::var("DBCLIENT_TEST_INFLUXDB_TOKEN").unwrap_or_default(),
    })
}

macro_rules! server_or_skip {
    () => {
        match server() {
            Some(server) => server,
            None => {
                eprintln!("skipped: DBCLIENT_TEST_INFLUXDB_URL is not set");
                return;
            }
        }
    };
}

impl Server {
    fn config(&self, database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            driver: "influxdb".into(),
            host: Some(self.host.clone()),
            port: Some(self.port),
            password: Some(self.token.clone()),
            database: database.map(str::to_owned),
            ..ConnectionConfig::default()
        }
    }

    async fn connect(&self) -> Box<dyn Connection> {
        InfluxDbDriver
            .connect(&self.config(Some("sensors")))
            .await
            .unwrap()
    }
}

fn sql(statement: &str) -> QueryRequest {
    QueryRequest {
        statement: statement.into(),
        max_rows: None,
    }
}

fn page(table: &str, limit: u32, offset: u64) -> TableRequest {
    TableRequest {
        namespace: Some("sensors".into()),
        table: table.into(),
        limit,
        offset,
        order_by: Vec::new(),
        filter: None,
    }
}

#[test]
fn driver_info() {
    let info = InfluxDbDriver.info();
    assert_eq!(info.id, "influxdb");
    assert_eq!(info.model, DataModel::Relational);
    assert!(info.capabilities.sql);
    assert!(!info.capabilities.transactions);
}

#[tokio::test]
async fn refuses_wrong_tokens_and_unknown_databases() {
    let server = server_or_skip!();
    let mut config = server.config(None);
    config.password = Some("apiv3_wrong".into());
    let Err(Error::Connect(message)) = InfluxDbDriver.connect(&config).await else {
        panic!("a wrong token must be refused");
    };
    assert!(message.contains("token"), "{message}");
    let Err(Error::Connect(message)) = InfluxDbDriver.connect(&server.config(Some("nope"))).await
    else {
        panic!("an unknown database must be refused");
    };
    assert!(message.contains("nope"), "{message}");
    // Without a database, the first one is chosen.
    let connection = InfluxDbDriver.connect(&server.config(None)).await.unwrap();
    assert_eq!(
        connection.current_namespace().await.unwrap().as_deref(),
        Some("garden")
    );
}

#[tokio::test]
async fn lists_and_describes() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let version = connection.server_version().await.unwrap().unwrap();
    assert!(
        version.starts_with('3') && version.contains("(Core)"),
        "{version}"
    );
    assert_eq!(
        connection.current_namespace().await.unwrap().as_deref(),
        Some("sensors")
    );
    let names: Vec<String> = connection
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    // The server's own `_internal` is hidden.
    assert_eq!(names, ["garden", "sensors"]);

    let objects = connection.list_objects(None).await.unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].name, "home");
    assert_eq!(objects[0].kind, ObjectKind::Table);
    let garden = connection.list_objects(Some("garden")).await.unwrap();
    assert_eq!(garden[0].name, "soil");

    let description = connection.describe_object(None, "home").await.unwrap();
    let columns: Vec<(&str, Option<&str>)> = description
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.comment.as_deref()))
        .collect();
    assert_eq!(
        columns,
        [
            ("city", Some("tag")),
            ("hum", Some("field")),
            ("note", Some("field")),
            ("ok", Some("field")),
            ("room", Some("tag")),
            ("temp", Some("field")),
            ("time", Some("time")),
        ]
    );
    assert!(description.columns.iter().all(|c| !c.primary_key));
    assert!(connection.describe_object(None, "nope").await.is_err());
}

#[tokio::test]
async fn pages_through_a_table_with_every_column() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let first = connection.fetch_rows(page("home", 12, 0)).await.unwrap();
    assert_eq!(first.rows.len(), 12);
    assert!(first.has_more);
    let note = first.columns.iter().position(|c| c.name == "note").unwrap();
    let time = first.columns.iter().position(|c| c.name == "time").unwrap();
    // The first point has no note: the cell is there, empty.
    assert_eq!(first.rows[0][note], Value::Null);
    assert_eq!(first.rows[1][note], Value::Text("Çağrı 1".into()));
    assert!(
        matches!(&first.rows[0][time], Value::Temporal(t) if t.starts_with("2023-11-14T22:13:20"))
    );

    let mut times = Vec::new();
    for offset in [0, 12, 24] {
        let page = connection
            .fetch_rows(page("home", 12, offset))
            .await
            .unwrap();
        times.extend(page.rows.iter().map(|r| format!("{:?}", r[time])));
        assert_eq!(page.has_more, offset < 24);
    }
    assert_eq!(times.len(), 30);
    let mut sorted = times.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, times, "pages are in time order without repeats");

    let mut filtered = page("home", 100, 0);
    filtered.filter = Some("city = 'İzmir' AND hum >= 50".into());
    filtered.order_by = vec![SortColumn {
        column: "hum".into(),
        descending: true,
    }];
    let result = connection.fetch_rows(filtered).await.unwrap();
    let hum = result.columns.iter().position(|c| c.name == "hum").unwrap();
    let values: Vec<Value> = result.rows.iter().map(|r| r[hum].clone()).collect();
    assert_eq!(
        values,
        [68, 66, 64, 62, 60, 58, 56, 54, 52, 50].map(Value::Int)
    );
    assert_eq!(
        connection
            .count_rows(None, "home", Some("city = 'Ankara'"))
            .await
            .unwrap(),
        15
    );
    assert_eq!(
        connection
            .count_rows(Some("garden"), "soil", None)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn runs_statements_and_use() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let results = connection
        .execute_all(sql(
            "SELECT room, avg(temp) AS t, count(*) AS n FROM home GROUP BY room ORDER BY room;\n\
             USE garden;\n\
             SELECT bed, moisture FROM soil",
        ))
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    let names: Vec<&str> = results[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["room", "t", "n"]);
    assert_eq!(results[0].rows[0][0], Value::Text("Mutfak".into()));
    assert_eq!(results[0].rows[0][2], Value::Int(15));
    assert!(matches!(results[0].rows[0][1], Value::Float(_)));
    assert_eq!(
        results[2].rows,
        [[Value::Text("north".into()), Value::Float(0.31)]]
    );
    assert_eq!(
        connection.current_namespace().await.unwrap().as_deref(),
        Some("garden")
    );

    let limited = connection
        .execute(QueryRequest {
            statement: "USE sensors; SELECT * FROM home".into(),
            max_rows: Some(5),
        })
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 5);
    assert!(limited.truncated);

    let missing = connection.execute(sql("USE nope")).await;
    assert!(matches!(missing, Err(Error::Query(m)) if m.contains("nope")));
}

#[tokio::test]
async fn errors_point_at_the_statement() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let text = "SELECT 1;\nSELECT *\n  FRM home";
    let Err(Error::Database {
        position, message, ..
    }) = connection.execute_all(sql(text)).await
    else {
        panic!("a syntax error must be reported");
    };
    let position = position.expect("the parser says where") as usize;
    // The position is inside the second statement.
    assert!(
        position > 10 && position <= text.chars().count() + 1,
        "{position}: {message}"
    );

    let Err(Error::Database { message, .. }) =
        connection.execute(sql("SELECT nope FROM home")).await
    else {
        panic!("an unknown column must be reported");
    };
    assert!(message.contains("nope"), "{message}");
    assert!(matches!(
        connection
            .apply_changes(&dbclient_core::ChangeSet {
                namespace: None,
                table: "home".into(),
                changes: Vec::new(),
            })
            .await,
        Err(Error::Unsupported(_))
    ));
    connection.ping().await.unwrap();
}
