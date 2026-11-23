//! Tests against real servers (see scripts/test-elasticsearch.sh). Each test
//! runs against Elasticsearch when `DBCLIENT_TEST_ELASTICSEARCH_URL` is set
//! and against OpenSearch when `DBCLIENT_TEST_OPENSEARCH_URL` is set.

use dbclient_core::document::{DocumentChange, DocumentStore, FindRequest};
use dbclient_core::{Connection, ConnectionConfig, Driver, Error, QueryRequest, Value};
use dbclient_driver_elasticsearch::ElasticsearchDriver;
use serde_json::json;

fn config_for(url: &str, user: Option<String>, password: Option<String>) -> ConnectionConfig {
    let rest = url
        .strip_prefix("http://")
        .expect("the test URL starts with http://");
    let (host, port) = rest.rsplit_once(':').expect("the test URL needs a port");
    ConnectionConfig {
        driver: "elasticsearch".into(),
        host: Some(host.into()),
        port: Some(port.trim_end_matches('/').parse().unwrap()),
        user,
        password,
        ..ConnectionConfig::default()
    }
}

/// (name, settings) of each server that is running.
fn servers() -> Vec<(&'static str, ConnectionConfig)> {
    let mut list = Vec::new();
    if let Ok(url) = std::env::var("DBCLIENT_TEST_ELASTICSEARCH_URL") {
        let user = std::env::var("DBCLIENT_TEST_ELASTICSEARCH_USER").ok();
        let password = std::env::var("DBCLIENT_TEST_ELASTICSEARCH_PASSWORD").ok();
        list.push(("Elasticsearch", config_for(&url, user, password)));
    }
    if let Ok(url) = std::env::var("DBCLIENT_TEST_OPENSEARCH_URL") {
        list.push(("OpenSearch", config_for(&url, None, None)));
    }
    if list.is_empty() {
        eprintln!(
            "skipped: neither DBCLIENT_TEST_ELASTICSEARCH_URL nor DBCLIENT_TEST_OPENSEARCH_URL is set"
        );
    }
    list
}

async fn console(conn: &dyn Connection, text: &str) -> Vec<dbclient_core::QueryResult> {
    conn.execute_all(QueryRequest {
        statement: text.into(),
        max_rows: None,
    })
    .await
    .unwrap()
}

/// A fresh index with a mapping and twelve documents.
async fn make_index(conn: &dyn Connection, index: &str) {
    let mut bulk = String::new();
    for i in 1..=12 {
        let city = ["İstanbul", "Ankara", "İzmir"][i % 3];
        bulk.push_str(&format!(
            "{{\"index\": {{\"_index\": \"{index}\", \"_id\": \"d{i}\"}}}}\n"
        ));
        bulk.push_str(&json!({"title": format!("Belge {i}"), "city": city, "year": 2014 + i, "tags": ["a", format!("t{}", i % 2)], "address": {"zip": format!("35{i:03}")}}).to_string());
        bulk.push('\n');
    }
    console(
        conn,
        &format!(
            "DELETE /{index}?ignore_unavailable=true\n\nPUT /{index}\n{{\"mappings\": {{\"properties\": {{\"title\": {{\"type\": \"text\"}}, \"city\": {{\"type\": \"keyword\"}}, \"year\": {{\"type\": \"integer\"}}, \"tags\": {{\"type\": \"keyword\"}}, \"address\": {{\"properties\": {{\"zip\": {{\"type\": \"keyword\"}}}}}}}}}}}}\n\nPOST /_bulk?refresh=true\n{bulk}"
        ),
    )
    .await;
}

fn find(database: &str, index: &str, filter: Option<&str>) -> FindRequest {
    FindRequest {
        database: database.into(),
        collection: index.into(),
        filter: filter.map(str::to_owned),
        projection: None,
        sort: Some(r#"{"year": 1}"#.into()),
        skip: 0,
        limit: 50,
    }
}

async fn ids(store: &dyn DocumentStore, request: FindRequest) -> Vec<String> {
    store
        .find(request)
        .await
        .unwrap()
        .documents
        .into_iter()
        .map(|d| d.document["_id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn connects_and_says_what_it_is() {
    for (name, config) in servers() {
        let conn = ElasticsearchDriver.connect(&config).await.unwrap();
        let version = conn.server_version().await.unwrap().unwrap();
        assert!(version.ends_with(&format!("({name})")), "{version}");
        conn.ping().await.unwrap();

        if config.user.is_some() {
            let mut wrong = config.clone();
            wrong.password = Some("not the password".into());
            assert!(
                matches!(
                    ElasticsearchDriver.connect(&wrong).await,
                    Err(Error::Connect(_))
                ),
                "{name}"
            );
            wrong.user = None;
            assert!(
                matches!(
                    ElasticsearchDriver.connect(&wrong).await,
                    Err(Error::Connect(_))
                ),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn finds_with_each_kind_of_filter() {
    for (name, config) in servers() {
        let conn = ElasticsearchDriver.connect(&config).await.unwrap();
        make_index(conn.as_ref(), "lw-find").await;
        let store = conn.document().unwrap();
        let cluster = store.list_databases().await.unwrap().remove(0).name;

        let indices: Vec<String> = store
            .list_collections(&cluster)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(
            indices.contains(&"lw-find".to_string()),
            "{name}: {indices:?}"
        );
        assert!(
            indices.iter().all(|i| !i.starts_with('.')),
            "{name}: system indices are hidden"
        );
        assert_eq!(
            store
                .collection_info(&cluster, "lw-find")
                .await
                .unwrap()
                .document_count,
            Some(12),
            "{name}"
        );

        // Shorthand, DSL and query string.
        assert_eq!(
            ids(
                store,
                find(&cluster, "lw-find", Some(r#"{"city": "İzmir"}"#))
            )
            .await,
            vec!["d2", "d5", "d8", "d11"],
            "{name}"
        );
        assert_eq!(
            ids(
                store,
                find(
                    &cluster,
                    "lw-find",
                    Some(r#"{"range": {"year": {"gte": 2024}}}"#)
                )
            )
            .await,
            vec!["d10", "d11", "d12"],
            "{name}"
        );
        assert_eq!(
            ids(
                store,
                find(&cluster, "lw-find", Some("city:Ankara AND year:>2020"))
            )
            .await,
            vec!["d7", "d10"],
            "{name}"
        );
        assert_eq!(
            ids(
                store,
                find(
                    &cluster,
                    "lw-find",
                    Some(r#"{"tags": ["t0"], "address.zip": "35004"}"#)
                )
            )
            .await,
            vec!["d4"],
            "{name}"
        );
        assert_eq!(
            store
                .count(&cluster, "lw-find", Some(r#"{"city": "İzmir"}"#))
                .await
                .unwrap(),
            4,
            "{name}"
        );

        // Paging and the sort.
        let mut page = find(&cluster, "lw-find", None);
        page.sort = Some(r#"{"year": -1}"#.into());
        page.limit = 5;
        page.skip = 5;
        let result = store.find(page).await.unwrap();
        let got: Vec<&str> = result
            .documents
            .iter()
            .map(|d| d.document["_id"].as_str().unwrap())
            .collect();
        assert_eq!(got, vec!["d7", "d6", "d5", "d4", "d3"], "{name}");
        assert!(result.has_more, "{name}");

        // Projection keeps _id.
        let mut slim = find(&cluster, "lw-find", Some(r#"{"ids": {"values": ["d1"]}}"#));
        slim.projection = Some(r#"{"title": 1}"#.into());
        let document = store.find(slim).await.unwrap().documents.remove(0).document;
        assert_eq!(document, json!({"_id": "d1", "title": "Belge 1"}), "{name}");

        // Mistakes come back as the field's error.
        let Err(Error::Database { code, .. }) = store
            .find(find(&cluster, "lw-find", Some("{\"city\": }")))
            .await
        else {
            panic!("{name}: bad JSON was accepted");
        };
        assert_eq!(code.as_deref(), Some("invalid_filter"));
        let Err(Error::Database { code, .. }) =
            store.find(find(&cluster, "lw-missing", None)).await
        else {
            panic!("{name}: a missing index was found");
        };
        assert_eq!(code.as_deref(), Some("index_not_found_exception"), "{name}");
    }
}

#[tokio::test]
async fn writes_documents() {
    for (name, config) in servers() {
        let conn = ElasticsearchDriver.connect(&config).await.unwrap();
        make_index(conn.as_ref(), "lw-write").await;
        let store = conn.document().unwrap();
        let cluster = store.list_databases().await.unwrap().remove(0).name;
        let one = |id: &str| {
            let mut request = find(
                &cluster,
                "lw-write",
                Some(&format!(r#"{{"ids": {{"values": ["{id}"]}}}}"#)),
            );
            request.sort = None;
            request
        };

        let added = store
            .write(
                &cluster,
                "lw-write",
                DocumentChange::Insert {
                    document: r#"{"title": "Yeni", "city": "Bursa"}"#.into(),
                },
            )
            .await
            .unwrap();
        let new_id = added.inserted_id.unwrap();
        assert!(new_id.is_string(), "{name}");
        // Written with a refresh: found at once.
        assert_eq!(
            store
                .count(&cluster, "lw-write", Some(r#"{"city": "Bursa"}"#))
                .await
                .unwrap(),
            1,
            "{name}"
        );

        store
            .write(
                &cluster,
                "lw-write",
                DocumentChange::Insert {
                    document: r#"{"_id": "mine", "title": "Benim"}"#.into(),
                },
            )
            .await
            .unwrap();
        assert!(
            store
                .write(
                    &cluster,
                    "lw-write",
                    DocumentChange::Insert {
                        document: r#"{"_id": "mine", "title": "Again"}"#.into()
                    }
                )
                .await
                .is_err(),
            "{name}: a taken id was overwritten"
        );

        let outcome = store
            .write(
                &cluster,
                "lw-write",
                DocumentChange::Update {
                    id: json!("d1"),
                    update:
                        r#"{"$set": {"address.zip": "06000", "tags.1": "z", "title": "Değişti"}}"#
                            .into(),
                },
            )
            .await
            .unwrap();
        assert_eq!((outcome.matched, outcome.modified), (1, 1), "{name}");
        let document = store
            .find(one("d1"))
            .await
            .unwrap()
            .documents
            .remove(0)
            .document;
        assert_eq!(document["address"]["zip"], json!("06000"), "{name}");
        assert_eq!(document["tags"], json!(["a", "z"]), "{name}");
        assert_eq!(document["title"], json!("Değişti"), "{name}");

        store
            .write(
                &cluster,
                "lw-write",
                DocumentChange::Replace {
                    id: json!("d2"),
                    document: r#"{"_id": "d2", "title": "Bütün"}"#.into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .find(one("d2"))
                .await
                .unwrap()
                .documents
                .remove(0)
                .document,
            json!({"_id": "d2", "title": "Bütün"}),
            "{name}"
        );

        assert_eq!(
            store
                .write(
                    &cluster,
                    "lw-write",
                    DocumentChange::Delete { id: json!("d3") }
                )
                .await
                .unwrap()
                .deleted,
            1,
            "{name}"
        );
        assert_eq!(
            store
                .write(
                    &cluster,
                    "lw-write",
                    DocumentChange::Delete { id: json!("d3") }
                )
                .await
                .unwrap()
                .deleted,
            0,
            "{name}"
        );
        let gone = store
            .write(
                &cluster,
                "lw-write",
                DocumentChange::Update {
                    id: json!("d3"),
                    update: r#"{"$set": {"a": 1}}"#.into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(gone.matched, 0, "{name}");
        assert!(
            store
                .write(
                    &cluster,
                    "lw-write",
                    DocumentChange::Update {
                        id: json!("d4"),
                        update: r#"{"$unset": {"a": 1}}"#.into()
                    }
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn the_query_tab_speaks_the_console() {
    for (name, config) in servers() {
        let conn = ElasticsearchDriver.connect(&config).await.unwrap();
        make_index(conn.as_ref(), "lw-console").await;

        let results = console(
            conn.as_ref(),
            "# two requests\nGET /lw-console/_search\n{\"query\": {\"term\": {\"city\": \"Ankara\"}}, \"sort\": [{\"year\": \"asc\"}]}\n\nGET /_cat/indices/lw-console?format=json&h=index,docs.count",
        )
        .await;
        assert_eq!(results.len(), 2, "{name}");
        let names: Vec<&str> = results[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["_id", "_index", "_score", "_source"], "{name}");
        assert_eq!(results[0].rows[0][0], Value::Text("d1".into()), "{name}");
        assert_eq!(
            results[1].rows[0][0],
            Value::Text("lw-console".into()),
            "{name}"
        );
        assert_eq!(results[1].rows[0][1], Value::Text("12".into()), "{name}");

        let Err(Error::Database { code, .. }) = conn
            .execute_all(QueryRequest {
                statement: "GET /lw-console/_search\n{\"query\": {\"nope\": {}}}".into(),
                max_rows: None,
            })
            .await
        else {
            panic!("{name}: a bad query was accepted");
        };
        assert!(code.is_some(), "{name}");

        let description = conn.describe_object(None, "lw-console").await.unwrap();
        let fields: Vec<&str> = description
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            fields.contains(&"_id") && fields.contains(&"address.zip") && fields.contains(&"year"),
            "{name}: {fields:?}"
        );
        let listed = conn.list_objects(None).await.unwrap();
        assert!(
            listed
                .iter()
                .any(|o| o.name == "lw-console" && o.row_estimate == Some(12)),
            "{name}"
        );
    }
}
