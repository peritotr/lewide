//! Tests against a real server. They run only when `DBCLIENT_TEST_COUCHBASE_URL`
//! is set, for example `http://127.0.0.1:8093` (the query service);
//! `scripts/test-couchbase.sh up` starts a one-node cluster, loads the sample
//! documents these tests read (`shop.sales.orders`, `shop._default._default`)
//! and prints the variables.
//!
//! Tests that write use the empty `shop.sales.drafts` and remove what they
//! add, so the tests that read can run beside them.

use dbclient_core::document::{DocumentChange, FindRequest};
use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, QueryRequest, Value,
};
use dbclient_driver_couchbase::CouchbaseDriver;
use serde_json::json;

struct Server {
    host: String,
    port: u16,
    user: String,
    password: String,
}

fn server() -> Option<Server> {
    let url = std::env::var("DBCLIENT_TEST_COUCHBASE_URL").ok()?;
    let rest = url.trim_end_matches('/').trim_start_matches("http://");
    let (host, port) = rest.rsplit_once(':').expect("the test URL has a port");
    Some(Server {
        host: host.to_owned(),
        port: port.parse().expect("a port number"),
        user: std::env::var("DBCLIENT_TEST_COUCHBASE_USER").unwrap_or_default(),
        password: std::env::var("DBCLIENT_TEST_COUCHBASE_PASSWORD").unwrap_or_default(),
    })
}

macro_rules! server_or_skip {
    () => {
        match server() {
            Some(server) => server,
            None => {
                eprintln!("skipped: DBCLIENT_TEST_COUCHBASE_URL is not set");
                return;
            }
        }
    };
}

impl Server {
    fn config(&self) -> ConnectionConfig {
        ConnectionConfig {
            driver: "couchbase".into(),
            host: Some(self.host.clone()),
            port: Some(self.port),
            user: Some(self.user.clone()),
            password: Some(self.password.clone()),
            ..ConnectionConfig::default()
        }
    }

    async fn connect(&self) -> Box<dyn Connection> {
        CouchbaseDriver.connect(&self.config()).await.unwrap()
    }
}

fn find(filter: Option<&str>, sort: Option<&str>, skip: u64, limit: u32) -> FindRequest {
    FindRequest {
        database: "shop".into(),
        collection: "sales.orders".into(),
        filter: filter.map(str::to_owned),
        projection: None,
        sort: sort.map(str::to_owned),
        skip,
        limit,
        pipeline: None,
    }
}

fn ids(page: &dbclient_core::document::FindPage) -> Vec<String> {
    page.documents
        .iter()
        .map(|d| d.document["_id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn driver_info() {
    let info = CouchbaseDriver.info();
    assert_eq!(info.id, "couchbase");
    assert_eq!(info.model, DataModel::Document);
    assert!(info.capabilities.sql);
}

#[tokio::test]
async fn refuses_wrong_passwords_and_unknown_buckets() {
    let server = server_or_skip!();
    let mut config = server.config();
    config.password = Some("wrong".into());
    let Err(Error::Connect(message)) = CouchbaseDriver.connect(&config).await else {
        panic!("a wrong password must be refused");
    };
    assert!(message.contains("not authorized"), "{message}");
    let mut config = server.config();
    config.database = Some("nope".into());
    let Err(Error::Connect(message)) = CouchbaseDriver.connect(&config).await else {
        panic!("an unknown bucket must be refused");
    };
    assert!(message.contains("nope"), "{message}");
}

#[tokio::test]
async fn lists_buckets_and_collections() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let version = connection.server_version().await.unwrap().unwrap();
    assert!(
        version.starts_with("8.") && version.ends_with("(Community)"),
        "{version}"
    );
    assert_eq!(
        connection.current_namespace().await.unwrap().as_deref(),
        Some("shop")
    );
    let store = connection.document().unwrap();
    let buckets: Vec<String> = store
        .list_databases()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(buckets, ["shop"]);
    let collections = store.list_collections("shop").await.unwrap();
    let names: Vec<&str> = collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["_default._default", "sales.drafts", "sales.orders"]);
    // `sales.drafts` is written by another test meanwhile; the other two stay as loaded.
    assert_eq!(collections[0].document_count, Some(1));
    assert_eq!(collections[2].document_count, Some(25));
    let objects = connection.list_objects(Some("shop")).await.unwrap();
    assert!(objects.iter().all(|o| o.kind == ObjectKind::Collection));
    assert!(store.collection_info("shop", "orders").await.is_err());
}

#[tokio::test]
async fn finds_pages_filters_and_sorts() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.document().unwrap();
    let first = store.find(find(None, None, 0, 10)).await.unwrap();
    assert!(first.has_more);
    assert_eq!(ids(&first)[..3], ["o01", "o02", "o03"]);
    let document = &first.documents[0].document;
    // The key comes first; values stay plain JSON.
    assert_eq!(document.as_object().unwrap().keys().next().unwrap(), "_id");
    assert_eq!(document["city"], "İzmir");
    assert_eq!(document["total"], json!(1.5));
    let last = store.find(find(None, None, 20, 10)).await.unwrap();
    assert_eq!(ids(&last), ["o21", "o22", "o23", "o24", "o25"]);
    assert!(!last.has_more);

    let izmir = store
        .find(find(
            Some(r#"{"city": "İzmir", "total": {"$gte": 10}}"#),
            Some(r#"{"total": -1}"#),
            0,
            100,
        ))
        .await
        .unwrap();
    assert_eq!(
        ids(&izmir),
        ["o25", "o23", "o21", "o19", "o17", "o15", "o13", "o11"]
    );
    // A list field matches when it holds the value; null matches a missing field.
    let holds = store
        .find(find(Some(r#"{"items": 7}"#), None, 0, 100))
        .await
        .unwrap();
    assert_eq!(ids(&holds), ["o06", "o07"]);
    let without_note = store
        .find(find(Some(r#"{"note": null}"#), None, 0, 100))
        .await
        .unwrap();
    assert_eq!(ids(&without_note), ["o05", "o10", "o15", "o20", "o25"]);
    // SQL++ text works as it is written after WHERE.
    let text = store
        .find(find(
            Some("city = \"Ankara\" AND total < 5"),
            Some("total DESC"),
            0,
            100,
        ))
        .await
        .unwrap();
    assert_eq!(ids(&text), ["o04", "o02"]);

    let mut projected = find(
        Some(r#"{"_id": "o03"}"#.replace("_id", "city").as_str()),
        None,
        0,
        1,
    );
    projected.filter = None;
    projected.projection = Some(r#"{"city": 1}"#.into());
    let page = store.find(projected).await.unwrap();
    assert_eq!(
        page.documents[0].document,
        json!({"_id": "o01", "city": "İzmir"})
    );

    assert_eq!(
        store
            .count("shop", "sales.orders", Some(r#"{"city": "Ankara"}"#))
            .await
            .unwrap(),
        12
    );
    let Err(Error::Database { code, .. }) = store.find(find(Some("{\"a\": }"), None, 0, 1)).await
    else {
        panic!("a filter that is not JSON must say so");
    };
    assert_eq!(code.as_deref(), Some("invalid_filter"));
    assert!(store.find(find(Some("nope ="), None, 0, 1)).await.is_err());
}

#[tokio::test]
async fn writes_with_cas_checks() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.document().unwrap();
    let write = |change| store.write("shop", "sales.drafts", change);
    let _ = write(DocumentChange::Delete { id: json!("t-1") }).await;

    let inserted = write(DocumentChange::Insert {
        document: r#"{"_id": "t-1", "city": "Eskişehir", "total": {"$numberDouble": "2.5"}, "items": [1, 2], "draft": true}"#.into(),
    })
    .await
    .unwrap();
    assert_eq!(inserted.inserted_id, Some(json!("t-1")));
    // A key that is taken is refused, not overwritten.
    assert!(
        write(DocumentChange::Insert {
            document: r#"{"_id": "t-1"}"#.into()
        })
        .await
        .is_err()
    );

    let updated = write(DocumentChange::Update {
        id: json!("t-1"),
        update: r#"{"$set": {"total": {"$numberDouble": "3.75"}, "items.1": 9, "address.city": "İzmir"}}"#.into(),
    })
    .await
    .unwrap();
    assert_eq!((updated.matched, updated.modified), (1, 1));
    let read = |filter: &'static str| {
        let mut request = find(Some(filter), None, 0, 1);
        request.collection = "sales.drafts".into();
        store.find(request)
    };
    let doc = read(r#"{"city": "Eskişehir"}"#).await.unwrap().documents[0]
        .document
        .clone();
    assert_eq!(doc["total"], json!(3.75));
    assert_eq!(doc["items"], json!([1, 9]));
    assert_eq!(doc["address"], json!({"city": "İzmir"}));

    // A replace drops the fields it leaves out.
    let replaced = write(DocumentChange::Replace {
        id: json!("t-1"),
        document: r#"{"_id": "t-1", "city": "Eskişehir", "total": 4}"#.into(),
    })
    .await
    .unwrap();
    assert_eq!(replaced.matched, 1);
    let doc = read(r#"{"city": "Eskişehir"}"#).await.unwrap().documents[0]
        .document
        .clone();
    assert_eq!(doc, json!({"_id": "t-1", "city": "Eskişehir", "total": 4}));

    // A missing key is not an error: nothing matched.
    let missing = write(DocumentChange::Update {
        id: json!("t-none"),
        update: r#"{"$set": {"a": 1}}"#.into(),
    })
    .await
    .unwrap();
    assert_eq!(missing.matched, 0);

    let deleted = write(DocumentChange::Delete { id: json!("t-1") })
        .await
        .unwrap();
    assert_eq!(deleted.deleted, 1);
    assert_eq!(
        write(DocumentChange::Delete { id: json!("t-1") })
            .await
            .unwrap()
            .deleted,
        0
    );

    // Without an _id the server makes a key.
    let made = write(DocumentChange::Insert {
        document: r#"{"city": "Bursa"}"#.into(),
    })
    .await
    .unwrap();
    let key = made.inserted_id.unwrap();
    assert!(key.as_str().is_some_and(|k| k.len() == 36), "{key}");
    write(DocumentChange::Delete { id: key }).await.unwrap();
    assert_eq!(store.count("shop", "sales.drafts", None).await.unwrap(), 0);
}

#[tokio::test]
async fn runs_statements() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let results = connection
        .execute_all(QueryRequest {
            statement: "SELECT o.city, COUNT(*) AS n FROM shop.sales.orders o GROUP BY o.city ORDER BY o.city;\n\
                        SELECT RAW META(o).id FROM shop.sales.orders o WHERE o.total > 25 ;\n\
                        UPDATE shop.sales.orders SET touched = true WHERE META().id = 'o01';\n\
                        UPDATE shop.sales.orders UNSET touched WHERE META().id = 'o01'"
                .into(),
            max_rows: None,
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 4);
    let names: Vec<&str> = results[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["city", "n"]);
    assert_eq!(
        results[0].rows[0],
        [Value::Text("Ankara".into()), Value::Int(12)]
    );
    assert_eq!(results[1].columns[0].name, "value");
    assert_eq!(results[1].rows, [[Value::Text("o25".into())]]);
    assert_eq!(results[2].rows_affected, Some(1));

    let text = "SELECT 1;\nSELECT *\n  FRM shop";
    let Err(Error::Database { position, code, .. }) = connection
        .execute_all(QueryRequest {
            statement: text.into(),
            max_rows: None,
        })
        .await
    else {
        panic!("a syntax error must be reported");
    };
    assert_eq!(code.as_deref(), Some("3000"));
    let position = position.expect("the server says where") as usize;
    assert!(
        position > 10 && position <= text.chars().count() + 1,
        "{position}"
    );
    connection.ping().await.unwrap();
}
