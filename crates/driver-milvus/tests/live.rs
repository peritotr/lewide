//! Tests against a real server. They run only when `DBCLIENT_TEST_MILVUS_URL`
//! is set, for example `http://127.0.0.1:19530`; `scripts/test-milvus.sh up`
//! starts a server, loads the sample collections these tests read (`articles`
//! and `products`) and prints the variables. `DBCLIENT_TEST_MILVUS_TOKEN` is
//! `user:password` of that server.
//!
//! The driver only reads, so the tests never change the collections.

use dbclient_core::vector::{
    Condition, ConditionOp, PointId, ScrollRequest, SearchQuery, SearchRequest, VectorData,
    VectorKind,
};
use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, TlsConfig,
};
use dbclient_driver_milvus::MilvusDriver;
use serde_json::{Value, json};

struct Server {
    host: String,
    port: u16,
    user: String,
    password: String,
}

fn server() -> Option<Server> {
    let url = std::env::var("DBCLIENT_TEST_MILVUS_URL").ok()?;
    let token = std::env::var("DBCLIENT_TEST_MILVUS_TOKEN").unwrap_or_default();
    let (user, password) = token.split_once(':').unwrap_or(("", ""));
    let rest = url.trim_end_matches('/').trim_start_matches("http://");
    let (host, port) = rest.rsplit_once(':').expect("the test URL has a port");
    Some(Server {
        host: host.to_owned(),
        port: port.parse().expect("a port number"),
        user: user.to_owned(),
        password: password.to_owned(),
    })
}

macro_rules! server_or_skip {
    () => {
        match server() {
            Some(server) => server,
            None => {
                eprintln!("skipped: DBCLIENT_TEST_MILVUS_URL is not set");
                return;
            }
        }
    };
}

impl Server {
    fn config(&self) -> ConnectionConfig {
        ConnectionConfig {
            driver: "milvus".into(),
            host: Some(self.host.clone()),
            port: Some(self.port),
            user: Some(self.user.clone()),
            password: Some(self.password.clone()),
            tls: TlsConfig::default(),
            ..ConnectionConfig::default()
        }
    }

    async fn connect(&self) -> Box<dyn Connection> {
        MilvusDriver.connect(&self.config()).await.unwrap()
    }
}

fn condition(field: &str, op: ConditionOp, value: Value) -> Condition {
    Condition {
        field: field.into(),
        op,
        value,
    }
}

fn search(collection: &str, query: SearchQuery) -> SearchRequest {
    SearchRequest {
        collection: collection.into(),
        query,
        using: None,
        limit: 5,
        min_score: None,
        filter: Vec::new(),
        with_payload: true,
        with_vector: false,
    }
}

#[test]
fn driver_info() {
    let info = MilvusDriver.info();
    assert_eq!(info.id, "milvus");
    assert_eq!(info.model, DataModel::Vector);
    assert!(info.capabilities.vector_search);
    assert!(!info.desktop_only);
}

#[tokio::test]
async fn refuses_wrong_credentials_and_unknown_databases() {
    let server = server_or_skip!();
    let mut config = server.config();
    config.password = Some("wrong".into());
    let Err(Error::Connect(message)) = MilvusDriver.connect(&config).await else {
        panic!("a wrong password must be refused");
    };
    assert!(message.contains("password"), "{message}");

    let mut config = server.config();
    config.database = Some("nope".into());
    let Err(Error::Connect(message)) = MilvusDriver.connect(&config).await else {
        panic!("an unknown database must be refused");
    };
    assert!(message.contains("database"), "{message}");
}

#[tokio::test]
async fn lists_and_describes_collections() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let objects = connection.list_objects(None).await.unwrap();
    let articles = objects
        .iter()
        .find(|o| o.name == "articles")
        .expect("articles");
    assert_eq!(articles.kind, ObjectKind::Collection);
    assert_eq!(articles.row_estimate, Some(40));
    assert!(objects.iter().any(|o| o.name == "products"));

    let store = connection.vector().unwrap();
    let info = store.collection_info("articles").await.unwrap();
    assert_eq!(info.status.as_deref(), Some("green"));
    assert_eq!(info.points, Some(40));
    assert_eq!(info.vectors.len(), 1);
    let vector = &info.vectors[0];
    assert_eq!(vector.name.as_deref(), Some("embedding"));
    assert_eq!(vector.kind, VectorKind::Dense);
    assert_eq!(vector.size, Some(4));
    assert_eq!(vector.distance.as_deref(), Some("COSINE"));
    assert!(vector.higher_is_better);
    let fields: Vec<&str> = info
        .payload_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, ["title", "year", "tags"]);

    let products = store.collection_info("products").await.unwrap();
    let metrics: Vec<(Option<&str>, Option<&str>, bool)> = products
        .vectors
        .iter()
        .map(|v| (v.name.as_deref(), v.distance.as_deref(), v.higher_is_better))
        .collect();
    assert_eq!(
        metrics,
        [
            (Some("image"), Some("L2"), false),
            (Some("text"), Some("IP"), true)
        ]
    );
}

#[tokio::test]
async fn searches_by_vector_with_a_filter() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.vector().unwrap();
    let mut request = search(
        "articles",
        SearchQuery::Vector {
            values: vec![1.0, 1.0, 1.0, 1.0],
        },
    );
    request.filter = vec![
        condition("city", ConditionOp::Eq, json!("İzmir")),
        condition("year", ConditionOp::Gt, json!(2003)),
        condition("tags", ConditionOp::Eq, json!("t1")),
    ];
    let result = store.search(request).await.unwrap();
    assert!(!result.hits.is_empty());
    for (i, hit) in result.hits.iter().enumerate() {
        assert_eq!(hit.rank as usize, i + 1);
        let payload = hit.point.payload.as_ref().unwrap();
        assert_eq!(payload["city"], "İzmir");
        assert!(payload["year"].as_i64().unwrap() > 2003);
        // Lists come as plain JSON lists.
        assert!(payload["tags"].as_array().unwrap().contains(&json!("t1")));
        assert!(payload["title"].as_str().unwrap().ends_with("Çağrı"));
        assert!(hit.point.vectors.is_empty());
    }
    assert!(result.hits.windows(2).all(|w| w[0].score >= w[1].score));

    // A minimum score on a similarity drops the weaker hits and says how many.
    let all = search(
        "articles",
        SearchQuery::Vector {
            values: vec![1.0, 1.0, 1.0, 1.0],
        },
    );
    let scores: Vec<f64> = store
        .search(all.clone())
        .await
        .unwrap()
        .hits
        .iter()
        .map(|h| h.score)
        .collect();
    let mut limited = all;
    limited.min_score = Some(scores[2]);
    let result = store.search(limited).await.unwrap();
    assert_eq!(
        result.hits.len() + result.below_min_score as usize,
        scores.len()
    );
    assert!(result.hits.iter().all(|h| h.score >= scores[2]));
}

#[tokio::test]
async fn searches_by_point_on_a_named_vector_with_text_keys() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.vector().unwrap();
    // "007" must stay text: read as a number it would be 7.
    let id: PointId = serde_json::from_value(json!("007")).unwrap();
    let mut request = search("products", SearchQuery::Point { id });
    request.using = Some("image".into());
    request.with_vector = true;
    let result = store.search(request.clone()).await.unwrap();
    let ids: Vec<String> = result.hits.iter().map(|h| h.point.id.to_string()).collect();
    // The point itself is left out; both others are at squared distance 2.
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&"007".to_string()));
    assert!(result.hits.iter().all(|h| h.score == 2.0));
    assert!(
        result.hits[0]
            .point
            .vectors
            .iter()
            .any(|v| v.name.as_deref() == Some("text"))
    );

    // A distance: the minimum score is the farthest a hit may be.
    request.min_score = Some(1.0);
    let result = store.search(request).await.unwrap();
    assert!(result.hits.is_empty());
    assert_eq!(result.below_min_score, 2);

    let point = store
        .get_point("products", &PointId::Uuid("007".into()))
        .await
        .unwrap();
    assert_eq!(point.payload.unwrap()["price"], json!(9.5));
    let image = point
        .vectors
        .iter()
        .find(|v| v.name.as_deref() == Some("image"))
        .unwrap();
    assert_eq!(
        image.data,
        VectorData::Dense {
            values: vec![0.0, 0.0, 1.0]
        }
    );

    let missing = store
        .get_point("products", &PointId::Uuid("nope".into()))
        .await;
    assert!(matches!(missing, Err(Error::Query(m)) if m.contains("nope")));
}

#[tokio::test]
async fn scrolls_in_key_order() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.vector().unwrap();
    let mut request = ScrollRequest {
        collection: "articles".into(),
        limit: 15,
        cursor: None,
        filter: vec![condition("city", ConditionOp::Ne, json!("Ankara"))],
        with_payload: true,
        with_vector: false,
    };
    let mut ids = Vec::new();
    loop {
        let page = store.scroll(request.clone()).await.unwrap();
        assert!(page.points.len() <= 15);
        ids.extend(
            page.points
                .iter()
                .map(|p| p.id.to_string().parse::<u64>().unwrap()),
        );
        match page.next {
            Some(next) => request.cursor = Some(next),
            None => break,
        }
    }
    // The odd ids are in İzmir: every one once, in order.
    assert_eq!(ids, (1..=40).filter(|i| i % 2 == 1).collect::<Vec<u64>>());

    let error = store
        .scroll(ScrollRequest {
            filter: vec![condition("title.x", ConditionOp::Eq, json!(1))],
            ..request
        })
        .await;
    assert!(matches!(error, Err(Error::Query(_))));
}

#[tokio::test]
async fn server_errors_carry_milvus_codes() {
    let server = server_or_skip!();
    let connection = server.connect().await;
    let store = connection.vector().unwrap();
    let mut request = search(
        "articles",
        SearchQuery::Vector {
            values: vec![1.0, 1.0, 1.0, 1.0],
        },
    );
    request.using = Some("nope".into());
    let Err(Error::Database { code, message, .. }) = store.search(request).await else {
        panic!("searching an unknown field must fail");
    };
    assert!(code.is_some());
    assert!(message.contains("nope"), "{message}");
    assert!(store.collection_info("missing").await.is_err());
    connection.ping().await.unwrap();
}
