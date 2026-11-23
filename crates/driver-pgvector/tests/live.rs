//! Tests against a real server. They run only when `DBCLIENT_TEST_PGVECTOR_URL`
//! is set (see scripts/test-pgvector.sh), and share one database, so each test
//! uses tables of its own.

use std::collections::BTreeMap;

use dbclient_core::vector::{
    Condition, ConditionOp, PointId, ScrollRequest, SearchQuery, SearchRequest, VectorData,
};
use dbclient_core::{Connection, ConnectionConfig, Driver, Error, QueryRequest};
use dbclient_driver_pgvector::PgvectorDriver;
use serde_json::json;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_PGVECTOR_URL").ok()?;
    let parsed: tokio_postgres::Config = url.parse().expect("invalid DBCLIENT_TEST_PGVECTOR_URL");
    let host = match parsed.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => panic!("the test URL needs a TCP host"),
    };
    Some(ConnectionConfig {
        driver: "pgvector".into(),
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

async fn connect(params: &[(&str, &str)]) -> Option<Box<dyn Connection>> {
    let Some(mut config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_PGVECTOR_URL is not set");
        return None;
    };
    config.params = params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<BTreeMap<_, _>>();
    Some(PgvectorDriver.connect(&config).await.unwrap())
}

async fn run(conn: &dyn Connection, sql: &str) {
    conn.execute_all(QueryRequest {
        statement: sql.into(),
        max_rows: None,
    })
    .await
    .unwrap();
}

fn search(collection: &str, query: SearchQuery) -> SearchRequest {
    SearchRequest {
        collection: collection.into(),
        query,
        using: None,
        limit: 10,
        min_score: None,
        filter: vec![],
        with_payload: true,
        with_vector: false,
    }
}

fn ids(hits: &[dbclient_core::vector::ScoredPoint]) -> Vec<String> {
    hits.iter().map(|h| h.point.id.to_string()).collect()
}

fn cond(field: &str, op: ConditionOp, value: serde_json::Value) -> Condition {
    Condition {
        field: field.into(),
        op,
        value,
    }
}

/// Six documents with 3-dimensional embeddings and no index: cosine.
async fn make_docs(conn: &dyn Connection, table: &str) {
    run(
        conn,
        &format!(
            "DROP TABLE IF EXISTS {table};
             CREATE TABLE {table} (id bigint PRIMARY KEY, title text, city text, year int, meta jsonb, embedding vector(3));
             INSERT INTO {table} VALUES
               (1, 'Bir', 'İzmir', 2021, '{{\"tag\": \"a\"}}', '[1,0,0]'),
               (2, 'İki', 'Ankara', 2022, '{{\"tag\": \"b\"}}', '[0.9,0.1,0]'),
               (3, 'Üç', 'İzmir', 2023, '{{\"tag\": \"a\"}}', '[0,1,0]'),
               (4, 'Dört', NULL, 2024, NULL, '[0,0,1]'),
               (5, 'Beş', 'Bursa', 2025, '{{\"tag\": \"c\"}}', '[0.7,0.7,0]'),
               (6, 'Altı', 'İzmir', 2026, '{{\"tag\": \"b\"}}', NULL);"
        ),
    )
    .await;
}

#[tokio::test]
async fn tables_with_vectors_are_collections() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_list").await;
    run(
        conn.as_ref(),
        "CREATE SCHEMA IF NOT EXISTS rag;
         DROP TABLE IF EXISTS rag.chunks, plain_table;
         CREATE TABLE rag.chunks (id uuid PRIMARY KEY, content text, emb vector(2));
         CREATE TABLE plain_table (id int PRIMARY KEY, name text);",
    )
    .await;
    let store = conn.vector().unwrap();

    let collections = store.list_collections().await.unwrap();
    assert!(
        collections.contains(&"docs_list".to_string()),
        "{collections:?}"
    );
    // Outside the current schema the schema is part of the name.
    assert!(
        collections.contains(&"rag.chunks".to_string()),
        "{collections:?}"
    );
    assert!(!collections.contains(&"plain_table".to_string()));

    let namespaces: Vec<String> = conn
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(namespaces.contains(&"public".to_string()) && namespaces.contains(&"rag".to_string()));
    let rag: Vec<String> = conn
        .list_objects(Some("rag"))
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.name)
        .collect();
    assert_eq!(rag, vec!["rag.chunks".to_string()]);

    let info = store.collection_info("docs_list").await.unwrap();
    assert_eq!(info.points, Some(6));
    assert_eq!(info.vectors.len(), 1);
    assert_eq!(info.vectors[0].name.as_deref(), Some("embedding"));
    assert_eq!(info.vectors[0].size, Some(3));
    assert_eq!(info.vectors[0].distance.as_deref(), Some("Cosine"));
    let fields: Vec<&str> = info
        .payload_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, vec!["title", "city", "year", "meta"]);

    // SQL still works on the same connection.
    run(conn.as_ref(), "SELECT 1").await;
    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.starts_with("0.") && version.contains("(PostgreSQL "),
        "{version}"
    );
}

#[tokio::test]
async fn searches_by_vector_and_by_point() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_search").await;
    let store = conn.vector().unwrap();

    let result = store
        .search(search(
            "docs_search",
            SearchQuery::Vector {
                values: vec![1.0, 0.0, 0.0],
            },
        ))
        .await
        .unwrap();
    // Point 6 has no embedding and is never a hit.
    assert_eq!(ids(&result.hits), vec!["1", "2", "5", "3", "4"]);
    assert!((result.hits[0].score - 1.0).abs() < 1e-6);
    assert!(result.hits[0].score > result.hits[1].score);
    let payload = result.hits[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["city"], json!("İzmir"));
    // The key and the vector are not part of the payload.
    assert!(payload.get("id").is_none() && payload.get("embedding").is_none());

    let mut request = search(
        "docs_search",
        SearchQuery::Vector {
            values: vec![1.0, 0.0, 0.0],
        },
    );
    request.min_score = Some(0.5);
    let result = store.search(request).await.unwrap();
    assert_eq!(ids(&result.hits), vec!["1", "2", "5"]);
    assert_eq!(result.below_min_score, 2);

    // "Find similar" leaves the point out.
    let result = store
        .search(search(
            "docs_search",
            SearchQuery::Point {
                id: PointId::Number(1),
            },
        ))
        .await
        .unwrap();
    assert_eq!(ids(&result.hits)[0], "2");
    assert!(!ids(&result.hits).contains(&"1".to_string()));

    let wrong = store
        .search(search(
            "docs_search",
            SearchQuery::Vector {
                values: vec![1.0, 0.0],
            },
        ))
        .await;
    assert!(matches!(wrong, Err(Error::Query(_))));
    let missing = store
        .search(search(
            "docs_search",
            SearchQuery::Point {
                id: PointId::Number(6),
            },
        ))
        .await;
    assert!(matches!(missing, Err(Error::Query(_))));
}

#[tokio::test]
async fn payload_filters() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_filter").await;
    let store = conn.vector().unwrap();
    let with = |filter: Vec<Condition>| {
        let mut request = search(
            "docs_filter",
            SearchQuery::Vector {
                values: vec![1.0, 0.0, 0.0],
            },
        );
        request.filter = filter;
        request
    };
    let hits = |filter| {
        let request = with(filter);
        async move {
            let mut found = ids(&store.search(request).await.unwrap().hits);
            found.sort();
            found
        }
    };

    assert_eq!(
        hits(vec![cond("city", ConditionOp::Eq, json!("İzmir"))]).await,
        vec!["1", "3"]
    );
    assert_eq!(
        hits(vec![cond("year", ConditionOp::Gte, json!(2023))]).await,
        vec!["3", "4", "5"]
    );
    assert_eq!(
        hits(vec![cond("meta.tag", ConditionOp::Eq, json!("a"))]).await,
        vec!["1", "3"]
    );
    assert_eq!(
        hits(vec![cond(
            "city",
            ConditionOp::In,
            json!(["Ankara", "Bursa"])
        )])
        .await,
        vec!["2", "5"]
    );
    assert_eq!(
        hits(vec![cond("city", ConditionOp::IsNull, json!(null))]).await,
        vec!["4"]
    );
    // Ne also matches rows without the value.
    assert_eq!(
        hits(vec![cond("city", ConditionOp::Ne, json!("İzmir"))]).await,
        vec!["2", "4", "5"]
    );
    assert_eq!(
        hits(vec![
            cond("city", ConditionOp::Eq, json!("İzmir")),
            cond("year", ConditionOp::Lt, json!(2022)),
        ])
        .await,
        vec!["1"]
    );
    // A quote in a value is data, not SQL.
    assert!(
        hits(vec![cond(
            "city",
            ConditionOp::Eq,
            json!("x'; DROP TABLE docs_filter; --")
        )])
        .await
        .is_empty()
    );
    assert_eq!(
        store.collection_info("docs_filter").await.unwrap().points,
        Some(6)
    );
}

#[tokio::test]
async fn reads_points_and_pages() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_scroll").await;
    let store = conn.vector().unwrap();

    let point = store
        .get_point("docs_scroll", &PointId::Number(5))
        .await
        .unwrap();
    assert_eq!(point.payload.as_ref().unwrap()["title"], json!("Beş"));
    assert_eq!(point.vectors.len(), 1);
    assert_eq!(
        point.vectors[0].data,
        VectorData::Dense {
            values: vec![0.7, 0.7, 0.0]
        }
    );
    assert!(
        store
            .get_point("docs_scroll", &PointId::Number(99))
            .await
            .is_err()
    );

    let page = |cursor| ScrollRequest {
        collection: "docs_scroll".into(),
        limit: 4,
        cursor,
        filter: vec![],
        with_payload: false,
        with_vector: false,
    };
    let first = store.scroll(page(None)).await.unwrap();
    assert_eq!(first.points.len(), 4);
    assert_eq!(first.next, Some(PointId::Number(4)));
    let second = store.scroll(page(first.next)).await.unwrap();
    let rest: Vec<String> = second.points.iter().map(|p| p.id.to_string()).collect();
    assert_eq!(rest, vec!["5", "6"]);
    assert_eq!(second.next, None);
}

#[tokio::test]
async fn the_metric_comes_from_the_index_or_the_option() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_l2").await;
    run(
        conn.as_ref(),
        "CREATE INDEX ON docs_l2 USING hnsw (embedding vector_l2_ops)",
    )
    .await;
    let store = conn.vector().unwrap();
    let info = store.collection_info("docs_l2").await.unwrap();
    assert_eq!(info.vectors[0].distance.as_deref(), Some("Euclid"));
    assert!(!info.vectors[0].higher_is_better);
    let result = store
        .search(search(
            "docs_l2",
            SearchQuery::Vector {
                values: vec![1.0, 0.0, 0.0],
            },
        ))
        .await
        .unwrap();
    // Distances: the best match has the lowest score.
    assert!(result.hits[0].score.abs() < 1e-6);
    assert!(result.hits[0].score < result.hits[1].score);

    // Without an index the option decides.
    let Some(conn) = connect(&[("metric", "ip")]).await else {
        return;
    };
    make_docs(conn.as_ref(), "docs_ip").await;
    let info = conn
        .vector()
        .unwrap()
        .collection_info("docs_ip")
        .await
        .unwrap();
    assert_eq!(info.vectors[0].distance.as_deref(), Some("Dot"));

    let mut config = test_config().unwrap();
    config.params.insert("metric".into(), "hamming".into());
    assert!(matches!(
        PgvectorDriver.connect(&config).await,
        Err(Error::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn a_table_without_a_single_key_is_refused() {
    let Some(conn) = connect(&[]).await else {
        return;
    };
    run(
        conn.as_ref(),
        "DROP TABLE IF EXISTS docs_nokey;
         CREATE TABLE docs_nokey (title text, embedding vector(2));",
    )
    .await;
    assert!(matches!(
        conn.vector().unwrap().collection_info("docs_nokey").await,
        Err(Error::Unsupported(_))
    ));
}
