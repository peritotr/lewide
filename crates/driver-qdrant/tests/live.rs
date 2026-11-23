//! Tests against a real server. They run only when `DBCLIENT_TEST_QDRANT_URL`
//! is set, for example `http://127.0.0.1:6333`; `scripts/test-qdrant.sh up`
//! starts a server and prints the variables. `DBCLIENT_TEST_QDRANT_API_KEY`
//! is the key of that server, when it asks for one.
//!
//! The driver only reads, so the tests create and remove their collections
//! with plain HTTP requests. Each test has a collection of its own.

use std::collections::BTreeSet;

use dbclient_core::vector::{
    Condition, ConditionOp, PointId, PointVector, ScrollRequest, SearchQuery, SearchRequest,
    VectorData, VectorKind, VectorStore,
};
use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, QueryRequest, TlsConfig,
    TlsMode,
};
use dbclient_driver_qdrant::QdrantDriver;
use serde_json::{Value, json};

const UUID: &str = "6f1e7f0a-0000-4000-8000-000000000001";

struct Server {
    url: String,
    key: Option<String>,
}

fn server() -> Option<Server> {
    let url = std::env::var("DBCLIENT_TEST_QDRANT_URL").ok()?;
    Some(Server {
        url: url.trim_end_matches('/').to_owned(),
        key: std::env::var("DBCLIENT_TEST_QDRANT_API_KEY").ok(),
    })
}

fn config_for(url: &str, key: Option<&str>) -> ConnectionConfig {
    let parsed = reqwest::Url::parse(url).expect("invalid test URL");
    ConnectionConfig {
        driver: "qdrant".into(),
        host: parsed.host_str().map(str::to_owned),
        port: parsed.port_or_known_default(),
        password: key.map(str::to_owned),
        tls: TlsConfig {
            mode: if parsed.scheme() == "https" {
                TlsMode::Verify
            } else {
                TlsMode::Off
            },
            ca_file: None,
        },
        ..ConnectionConfig::default()
    }
}

macro_rules! server_or_skip {
    () => {
        match server() {
            Some(server) => server,
            None => {
                eprintln!("skipped: DBCLIENT_TEST_QDRANT_URL is not set");
                return;
            }
        }
    };
}

impl Server {
    fn config(&self) -> ConnectionConfig {
        config_for(&self.url, self.key.as_deref())
    }

    async fn connect(&self) -> Box<dyn Connection> {
        QdrantDriver.connect(&self.config()).await.unwrap()
    }

    /// A request made by the test itself, not through the driver.
    async fn admin(&self, method: reqwest::Method, path: &str, body: Option<Value>) {
        // reqwest is built without a TLS provider of its own; it needs one
        // chosen even for plain HTTP.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut request = reqwest::Client::new().request(method, format!("{}{path}", self.url));
        if let Some(key) = &self.key {
            request = request.header("api-key", key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
    }

    async fn drop_collection(&self, name: &str) {
        let path = format!("/collections/{name}");
        self.admin(reqwest::Method::DELETE, &path, None).await;
    }

    /// Creates the collection anew, so a run that failed halfway leaves nothing behind.
    async fn create(&self, name: &str, vectors: Value, points: Value) {
        self.drop_collection(name).await;
        let path = format!("/collections/{name}");
        self.admin(reqwest::Method::PUT, &path, Some(vectors)).await;
        let path = format!("/collections/{name}/points?wait=true");
        let body = json!({ "points": points });
        self.admin(reqwest::Method::PUT, &path, Some(body)).await;
    }

    /// Five points in four dimensions. Seen from `[1, 0, 0, 0]` their cosine
    /// scores are 1, 0.994, 0.707, 0 and 0, for the ids 1, 2, 5, UUID and 4.
    async fn cities(&self, name: &str) {
        self.create(
            name,
            json!({ "vectors": { "size": 4, "distance": "Cosine" } }),
            json!([
                { "id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {
                    "city": "Berlin", "n": 5, "tags": ["a", "b"], "nested": { "x": 1 } } },
                { "id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {
                    "city": "Paris", "n": 10, "opt": null } },
                { "id": UUID, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {
                    "city": "Rome", "n": 2.5, "flag": true, "when": "2024-05-01T00:00:00Z" } },
                { "id": 4, "vector": [0.0, 0.0, 1.0, 0.0] },
                { "id": 5, "vector": [0.5, 0.5, 0.0, 0.0], "payload": {
                    "city": "Oslo", "n": 5.0, "when": "2023-01-01T00:00:00Z" } }
            ]),
        )
        .await;
    }
}

fn n(id: u64) -> PointId {
    PointId::Number(id)
}

fn uuid() -> PointId {
    PointId::Uuid(UUID.into())
}

fn by_vector(collection: &str, values: &[f32], limit: u32) -> SearchRequest {
    SearchRequest {
        collection: collection.into(),
        query: SearchQuery::Vector {
            values: values.to_vec(),
        },
        using: None,
        limit,
        min_score: None,
        filter: Vec::new(),
        with_payload: false,
        with_vector: false,
    }
}

fn page(collection: &str, limit: u32) -> ScrollRequest {
    ScrollRequest {
        collection: collection.into(),
        limit,
        cursor: None,
        filter: Vec::new(),
        with_payload: false,
        with_vector: false,
    }
}

fn cond(field: &str, op: ConditionOp, value: Value) -> Condition {
    Condition {
        field: field.into(),
        op,
        value,
    }
}

fn vectors(conn: &dyn Connection) -> &dyn VectorStore {
    conn.vector().expect("the driver has vector operations")
}

/// The HTTP status of a request the server rejected.
fn status(error: &Error) -> Option<&str> {
    match error {
        Error::Database { code, .. } => code.as_deref(),
        _ => None,
    }
}

#[tokio::test]
async fn connects_and_reports_the_version() {
    let server = server_or_skip!();
    let info = QdrantDriver.info();
    assert_eq!((info.id, info.name), ("qdrant", "Qdrant"));
    assert_eq!(info.model, DataModel::Vector);
    assert!(info.capabilities.vector_search && !info.capabilities.sql);
    assert_eq!(info.default_port, Some(6333));

    let conn = server.connect().await;
    conn.ping().await.unwrap();
    assert!(conn.is_alive().await);
    let version = conn.server_version().await.unwrap().unwrap();
    let mut parts = version.split('.');
    assert!(
        parts
            .next()
            .is_some_and(|major| major.parse::<u32>().is_ok())
            && parts.next().is_some(),
        "{version}"
    );
    assert_eq!(conn.current_namespace().await.unwrap(), None);
    assert!(conn.list_namespaces().await.unwrap().is_empty());
    assert!(!conn.in_transaction().await.unwrap());
    assert_eq!(conn.quote_ident("a b"), "a b");

    // The SQL-shaped operations have no meaning here.
    let statement = QueryRequest {
        statement: "SELECT 1".into(),
        max_rows: None,
    };
    assert!(matches!(
        conn.execute_all(statement).await,
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        conn.describe_object(None, "anything").await,
        Err(Error::Unsupported(_))
    ));
    conn.close().await.unwrap();
}

#[tokio::test]
async fn lists_collections() {
    let server = server_or_skip!();
    let name = "dbclient_test_list";
    server.cities(name).await;
    let conn = server.connect().await;

    let names = vectors(conn.as_ref()).list_collections().await.unwrap();
    assert!(names.iter().any(|c| c == name), "{names:?}");
    assert!(names.is_sorted());

    let objects = conn.list_objects(None).await.unwrap();
    let object = objects.iter().find(|o| o.name == name).unwrap();
    assert_eq!(object.kind, ObjectKind::Collection);
    assert_eq!(object.row_estimate, Some(5));

    server.drop_collection(name).await;
}

#[tokio::test]
async fn describes_a_collection() {
    let server = server_or_skip!();
    let name = "dbclient_test_info";
    server.cities(name).await;
    let path = format!("/collections/{name}/index?wait=true");
    let index = json!({ "field_name": "city", "field_schema": "keyword" });
    server.admin(reqwest::Method::PUT, &path, Some(index)).await;
    let conn = server.connect().await;

    let info = vectors(conn.as_ref()).collection_info(name).await.unwrap();
    assert_eq!(info.name, name);
    assert_eq!(info.points, Some(5));
    assert!(info.status.is_some());
    assert_eq!(info.vectors.len(), 1);
    let vector = &info.vectors[0];
    assert_eq!(vector.name, None);
    assert_eq!(vector.kind, VectorKind::Dense);
    assert_eq!(vector.size, Some(4));
    assert_eq!(vector.distance.as_deref(), Some("Cosine"));
    assert!(vector.higher_is_better);
    assert_eq!(info.payload_fields.len(), 1);
    assert_eq!(info.payload_fields[0].name, "city");
    assert_eq!(info.payload_fields[0].kind, "keyword");
    assert_eq!(info.payload_fields[0].points, Some(4));

    server.drop_collection(name).await;
}

#[tokio::test]
async fn searches_by_vector_with_top_k() {
    let server = server_or_skip!();
    let name = "dbclient_test_top_k";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let result = store
        .search(by_vector(name, &[1.0, 0.0, 0.0, 0.0], 3))
        .await
        .unwrap();
    let ids: Vec<_> = result.hits.iter().map(|h| h.point.id.clone()).collect();
    assert_eq!(ids, vec![n(1), n(2), n(5)]);
    let ranks: Vec<_> = result.hits.iter().map(|h| h.rank).collect();
    assert_eq!(ranks, vec![1, 2, 3]);
    assert!((result.hits[0].score - 1.0).abs() < 1e-5);
    assert!((result.hits[1].score - 0.99388).abs() < 1e-4);
    assert!((result.hits[2].score - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert_eq!(result.below_min_score, 0);

    // K larger than the collection returns every point.
    let all = store
        .search(by_vector(name, &[1.0, 0.0, 0.0, 0.0], 50))
        .await
        .unwrap();
    assert_eq!(all.hits.len(), 5);
    assert!(all.hits.windows(2).all(|w| w[0].score >= w[1].score));

    let none = store
        .search(by_vector(name, &[1.0, 0.0, 0.0, 0.0], 0))
        .await;
    assert!(matches!(none, Err(Error::Query(_))), "{none:?}");

    server.drop_collection(name).await;
}

#[tokio::test]
async fn leaves_out_hits_below_the_minimum_score() {
    let server = server_or_skip!();
    let name = "dbclient_test_min_score";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let mut request = by_vector(name, &[1.0, 0.0, 0.0, 0.0], 5);
    request.min_score = Some(0.8);
    let result = store.search(request.clone()).await.unwrap();
    let ids: Vec<_> = result.hits.iter().map(|h| h.point.id.clone()).collect();
    assert_eq!(ids, vec![n(1), n(2)]);
    assert_eq!(result.below_min_score, 3);

    // Only hits inside the top K are counted as left out.
    request.limit = 3;
    let result = store.search(request).await.unwrap();
    assert_eq!((result.hits.len(), result.below_min_score), (2, 1));

    server.drop_collection(name).await;
}

#[tokio::test]
async fn a_minimum_score_limits_the_distance_for_distance_metrics() {
    let server = server_or_skip!();
    let name = "dbclient_test_euclid";
    server
        .create(
            name,
            json!({ "vectors": { "size": 2, "distance": "Euclid" } }),
            json!([
                { "id": 1, "vector": [0.0, 0.0] },
                { "id": 2, "vector": [3.0, 4.0] },
                { "id": 3, "vector": [6.0, 8.0] }
            ]),
        )
        .await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let info = store.collection_info(name).await.unwrap();
    assert_eq!(info.vectors[0].distance.as_deref(), Some("Euclid"));
    assert!(!info.vectors[0].higher_is_better);

    let mut request = by_vector(name, &[0.0, 0.0], 3);
    request.min_score = Some(5.5);
    let result = store.search(request).await.unwrap();
    let hits: Vec<_> = result
        .hits
        .iter()
        .map(|h| (h.point.id.clone(), h.score))
        .collect();
    // Nearest first: the scores are distances and go up.
    assert_eq!(hits, vec![(n(1), 0.0), (n(2), 5.0)]);
    assert_eq!(result.below_min_score, 1);

    server.drop_collection(name).await;
}

#[tokio::test]
async fn returns_payload_and_vector_only_when_asked() {
    let server = server_or_skip!();
    let name = "dbclient_test_payload";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let mut request = by_vector(name, &[0.0, 0.0, 1.0, 0.0], 5);
    let bare = store.search(request.clone()).await.unwrap();
    assert!(
        bare.hits
            .iter()
            .all(|h| h.point.payload.is_none() && h.point.vectors.is_empty())
    );

    request.with_payload = true;
    let with_payload = store.search(request.clone()).await.unwrap();
    // Point 4 is the best match and has no payload: an empty object.
    assert_eq!(with_payload.hits[0].point.id, n(4));
    assert_eq!(with_payload.hits[0].point.payload, Some(json!({})));
    let berlin = with_payload
        .hits
        .iter()
        .find(|h| h.point.id == n(1))
        .unwrap();
    assert_eq!(
        berlin.point.payload,
        Some(json!({ "city": "Berlin", "n": 5, "tags": ["a", "b"], "nested": { "x": 1 } }))
    );
    assert!(berlin.point.vectors.is_empty());

    request.with_payload = false;
    request.with_vector = true;
    let with_vector = store.search(request).await.unwrap();
    assert_eq!(with_vector.hits[0].point.payload, None);
    assert_eq!(
        with_vector.hits[0].point.vectors,
        vec![PointVector {
            name: None,
            data: VectorData::Dense {
                values: vec![0.0, 0.0, 1.0, 0.0]
            }
        }]
    );

    server.drop_collection(name).await;
}

#[tokio::test]
async fn filters_by_payload() {
    use ConditionOp::*;
    let server = server_or_skip!();
    let name = "dbclient_test_filter";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let u = uuid();
    let cases: Vec<(Vec<Condition>, Vec<PointId>)> = vec![
        (vec![cond("city", Eq, json!("Berlin"))], vec![n(1)]),
        // A whole number finds the value stored as 5 and as 5.0.
        (vec![cond("n", Eq, json!(5))], vec![n(1), n(5)]),
        (vec![cond("n", Eq, json!(2.5))], vec![u.clone()]),
        (vec![cond("flag", Eq, json!(true))], vec![u.clone()]),
        // Any item of a list can match; nested fields are reached with a dot.
        (vec![cond("tags", Eq, json!("a"))], vec![n(1)]),
        (vec![cond("nested.x", Eq, json!(1))], vec![n(1)]),
        // "Not equal" includes the point that has no city at all.
        (
            vec![cond("city", Ne, json!("Berlin"))],
            vec![n(2), n(4), n(5), u.clone()],
        ),
        (vec![cond("n", Ne, json!(5))], vec![n(2), n(4), u.clone()]),
        (vec![cond("n", Gt, json!(5))], vec![n(2)]),
        (vec![cond("n", Gte, json!(5))], vec![n(1), n(2), n(5)]),
        (vec![cond("n", Lt, json!(5))], vec![u.clone()]),
        (vec![cond("n", Lte, json!(5))], vec![n(1), n(5), u.clone()]),
        (
            vec![cond("when", Gt, json!("2024-01-01T00:00:00Z"))],
            vec![u.clone()],
        ),
        (
            vec![cond("when", Lte, json!("2023-01-01T00:00:00Z"))],
            vec![n(5)],
        ),
        (
            vec![cond("city", In, json!(["Berlin", "Rome"]))],
            vec![n(1), u.clone()],
        ),
        (vec![cond("n", In, json!([5, 10]))], vec![n(1), n(2), n(5)]),
        (vec![cond("n", In, json!([2.5, "x"]))], vec![u.clone()]),
        (vec![cond("city", In, json!([]))], vec![]),
        (vec![cond("city", IsNull, Value::Null)], vec![n(4)]),
        // Stored as null on one point and missing on the others: all are null.
        (
            vec![cond("opt", IsNull, Value::Null)],
            vec![n(1), n(2), n(4), n(5), u.clone()],
        ),
        (
            vec![cond("city", IsNotNull, Value::Null)],
            vec![n(1), n(2), n(5), u.clone()],
        ),
        (vec![cond("opt", IsNotNull, Value::Null)], vec![]),
        // Several conditions must all hold.
        (
            vec![cond("city", Ne, json!("Berlin")), cond("n", Gte, json!(5))],
            vec![n(2), n(5)],
        ),
        (
            vec![
                cond("city", In, json!(["Berlin", "Paris", "Oslo"])),
                cond("n", Lte, json!(5)),
                cond("tags", IsNull, Value::Null),
            ],
            vec![n(5)],
        ),
    ];

    for (filter, expected) in cases {
        let expected: BTreeSet<String> = expected.iter().map(PointId::to_string).collect();

        let mut request = page(name, 50);
        request.filter = filter.clone();
        let listed = store.scroll(request).await.unwrap();
        let found: BTreeSet<String> = listed.points.iter().map(|p| p.id.to_string()).collect();
        assert_eq!(found, expected, "scroll with {filter:?}");

        // A search applies the same filter before ranking.
        let mut request = by_vector(name, &[1.0, 1.0, 1.0, 0.0], 50);
        request.filter = filter.clone();
        let searched = store.search(request).await.unwrap();
        let found: BTreeSet<String> = searched
            .hits
            .iter()
            .map(|h| h.point.id.to_string())
            .collect();
        assert_eq!(found, expected, "search with {filter:?}");
    }

    // A bound that is neither a number nor a date-time: the server says so.
    let mut request = page(name, 50);
    request.filter = vec![cond("n", Gt, json!("abc"))];
    let error = store.scroll(request).await.unwrap_err();
    assert_eq!(status(&error), Some("400"), "{error:?}");
    // A value no comparison can take is refused before anything is sent.
    let mut request = page(name, 50);
    request.filter = vec![cond("n", Eq, Value::Null)];
    assert!(matches!(store.scroll(request).await, Err(Error::Query(_))));

    server.drop_collection(name).await;
}

#[tokio::test]
async fn searches_by_point_id() {
    let server = server_or_skip!();
    let name = "dbclient_test_by_id";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let similar_to = |id: PointId| SearchRequest {
        query: SearchQuery::Point { id },
        ..by_vector(name, &[], 2)
    };

    // The point itself is not among its own neighbours.
    let result = store.search(similar_to(n(1))).await.unwrap();
    let ids: Vec<_> = result.hits.iter().map(|h| h.point.id.clone()).collect();
    assert_eq!(ids, vec![n(2), n(5)]);
    assert_eq!(result.hits[0].rank, 1);

    let result = store.search(similar_to(uuid())).await.unwrap();
    let ids: Vec<_> = result.hits.iter().map(|h| h.point.id.clone()).collect();
    assert_eq!(ids, vec![n(5), n(2)]);

    // A UUID point can be a hit too.
    let mut request = similar_to(n(5));
    request.limit = 4;
    let result = store.search(request).await.unwrap();
    assert!(result.hits.iter().any(|h| h.point.id == uuid()));

    let error = store.search(similar_to(n(999))).await.unwrap_err();
    assert_eq!(status(&error), Some("404"), "{error:?}");
    assert!(error.to_string().contains("999"), "{error}");

    server.drop_collection(name).await;
}

#[tokio::test]
async fn reads_one_point_with_its_vector() {
    let server = server_or_skip!();
    let name = "dbclient_test_point";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let point = store.get_point(name, &n(4)).await.unwrap();
    assert_eq!(point.id, n(4));
    assert_eq!(point.payload, Some(json!({})));
    assert_eq!(
        point.vectors,
        vec![PointVector {
            name: None,
            data: VectorData::Dense {
                values: vec![0.0, 0.0, 1.0, 0.0]
            }
        }]
    );

    let point = store.get_point(name, &uuid()).await.unwrap();
    assert_eq!(point.id, uuid());
    assert_eq!(point.payload.unwrap()["city"], "Rome");

    // A Cosine collection stores vectors at length 1, not as they were written.
    let point = store.get_point(name, &n(5)).await.unwrap();
    let VectorData::Dense { values } = &point.vectors[0].data else {
        panic!("expected a dense vector");
    };
    assert!(
        (values[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4
            && (values[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4
    );

    let error = store.get_point(name, &n(999)).await.unwrap_err();
    assert_eq!(status(&error), Some("404"), "{error:?}");
    let error = store
        .get_point(name, &PointId::Uuid("not-an-id".into()))
        .await
        .unwrap_err();
    assert_eq!(status(&error), Some("400"), "{error:?}");

    server.drop_collection(name).await;
}

#[tokio::test]
async fn scrolls_through_a_collection() {
    let server = server_or_skip!();
    let name = "dbclient_test_scroll";
    server.cities(name).await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let mut seen = Vec::new();
    let mut request = page(name, 2);
    request.with_payload = true;
    let mut pages = 0;
    loop {
        let result = store.scroll(request.clone()).await.unwrap();
        pages += 1;
        assert!(result.points.len() <= 2);
        assert!(result.points.iter().all(|p| p.payload.is_some()));
        seen.extend(result.points.into_iter().map(|p| p.id));
        match result.next {
            Some(next) => request.cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    // Numbers in order, then UUIDs: no point twice, none missing.
    assert_eq!(seen, vec![n(1), n(2), n(4), n(5), uuid()]);

    let mut request = page(name, 1);
    request.with_vector = true;
    let first = store.scroll(request).await.unwrap();
    assert_eq!(first.points[0].payload, None);
    assert_eq!(first.points[0].vectors.len(), 1);
    assert_eq!(first.next, Some(n(2)));

    assert!(matches!(
        store.scroll(page(name, 0)).await,
        Err(Error::Query(_))
    ));

    server.drop_collection(name).await;
}

#[tokio::test]
async fn searches_a_named_vector() {
    let server = server_or_skip!();
    let name = "dbclient_test_named";
    server
        .create(
            name,
            json!({
                "vectors": {
                    "text": { "size": 3, "distance": "Dot" },
                    "tokens": {
                        "size": 2, "distance": "Cosine",
                        "multivector_config": { "comparator": "max_sim" }
                    }
                },
                "sparse_vectors": { "words": {} }
            }),
            json!([
                { "id": 1, "vector": {
                    "text": [1.0, 0.0, 0.0],
                    "tokens": [[1.0, 0.0], [0.0, 1.0]],
                    "words": { "indices": [1, 7], "values": [0.5, 0.25] } } },
                { "id": 2, "vector": { "text": [0.0, 2.0, 0.0] } }
            ]),
        )
        .await;
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());

    let info = store.collection_info(name).await.unwrap();
    let specs: Vec<_> = info
        .vectors
        .iter()
        .map(|v| (v.name.as_deref(), v.kind, v.size, v.distance.as_deref()))
        .collect();
    assert_eq!(
        specs,
        vec![
            (Some("text"), VectorKind::Dense, Some(3), Some("Dot")),
            (Some("tokens"), VectorKind::Multi, Some(2), Some("Cosine")),
            (Some("words"), VectorKind::Sparse, None, None),
        ]
    );

    let mut request = by_vector(name, &[0.0, 1.0, 0.0], 2);
    request.using = Some("text".into());
    request.min_score = Some(1.0);
    let result = store.search(request.clone()).await.unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(
        (result.hits[0].point.id.clone(), result.hits[0].score),
        (n(2), 2.0)
    );
    assert_eq!(result.below_min_score, 1);

    // Without a name there is no vector to search in this collection.
    request.using = None;
    request.min_score = None;
    let error = store.search(request).await.unwrap_err();
    assert_eq!(status(&error), Some("400"), "{error:?}");

    let point = store.get_point(name, &n(1)).await.unwrap();
    let data = |vector: &str| {
        let found = point
            .vectors
            .iter()
            .find(|v| v.name.as_deref() == Some(vector));
        found.unwrap().data.clone()
    };
    assert_eq!(
        data("text"),
        VectorData::Dense {
            values: vec![1.0, 0.0, 0.0]
        }
    );
    assert_eq!(
        data("tokens"),
        VectorData::Multi {
            vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        }
    );
    assert_eq!(
        data("words"),
        VectorData::Sparse {
            indices: vec![1, 7],
            values: vec![0.5, 0.25]
        }
    );

    server.drop_collection(name).await;
}

#[tokio::test]
async fn a_vector_of_the_wrong_size_is_an_error() {
    let server = server_or_skip!();
    let name = "dbclient_test_dimension";
    server.cities(name).await;
    let conn = server.connect().await;

    let error = vectors(conn.as_ref())
        .search(by_vector(name, &[1.0, 0.0, 0.0], 3))
        .await
        .unwrap_err();
    assert_eq!(status(&error), Some("400"), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("expected dim: 4") && message.contains("got 3"),
        "{message}"
    );
    // The server refused the request; the connection is fine.
    assert!(conn.is_alive().await);

    server.drop_collection(name).await;
}

#[tokio::test]
async fn an_unknown_collection_is_an_error() {
    let server = server_or_skip!();
    let conn = server.connect().await;
    let store = vectors(conn.as_ref());
    let name = "dbclient_test_missing";

    let errors = [
        store.collection_info(name).await.unwrap_err(),
        store.search(by_vector(name, &[1.0], 3)).await.unwrap_err(),
        store.get_point(name, &n(1)).await.unwrap_err(),
        store.scroll(page(name, 3)).await.unwrap_err(),
    ];
    for error in errors {
        assert_eq!(status(&error), Some("404"), "{error:?}");
        assert!(error.to_string().contains(name), "{error}");
    }
    // A name with characters that mean something in a URL stays one path segment.
    let error = store.collection_info("a/../b c?d").await.unwrap_err();
    assert!(matches!(status(&error), Some("404" | "422")), "{error:?}");
}

#[tokio::test]
async fn a_wrong_api_key_is_refused() {
    let server = server_or_skip!();
    if server.key.is_none() {
        eprintln!("skipped: the test server has no API key");
        return;
    }
    for key in [Some("not-the-key"), None] {
        let config = config_for(&server.url, key);
        let Err(error) = QdrantDriver.connect(&config).await else {
            panic!("connected with {key:?}");
        };
        assert!(
            matches!(&error, Error::Connect(message) if message.contains("API key")),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn connects_through_a_route() {
    let server = server_or_skip!();
    let direct = server.config();
    let route = format!(
        "{}:{}",
        direct.host.as_deref().unwrap(),
        direct.port.unwrap()
    );
    // Neither host can be reached; only the route leads to the server.
    for host in ["qdrant.invalid", "192.0.2.10"] {
        let config = ConnectionConfig {
            host: Some(host.into()),
            port: Some(6333),
            route: Some(route.parse().expect("the test URL needs an IP address")),
            ..direct.clone()
        };
        let conn = QdrantDriver.connect(&config).await.unwrap();
        conn.ping().await.unwrap();
        assert!(conn.server_version().await.unwrap().is_some());
    }
}

// ---- TLS: a second server, started by `scripts/test-qdrant.sh tls` ----

/// `DBCLIENT_TEST_QDRANT_TLS_URL` (`https://127.0.0.1:PORT`) and the CA file
/// that signed its certificate. The certificate names `localhost`,
/// `127.0.0.1` and `192.0.2.10`.
fn tls_server() -> Option<(ConnectionConfig, String)> {
    let url = std::env::var("DBCLIENT_TEST_QDRANT_TLS_URL").ok()?;
    let ca = std::env::var("DBCLIENT_TEST_QDRANT_TLS_CA").ok()?;
    let key = std::env::var("DBCLIENT_TEST_QDRANT_API_KEY").ok();
    Some((config_for(&url, key.as_deref()), ca))
}

#[tokio::test]
async fn tls_modes() {
    let Some((base, ca)) = tls_server() else {
        eprintln!("skipped: DBCLIENT_TEST_QDRANT_TLS_URL is not set");
        return;
    };
    let with = |mode: TlsMode, ca_file: Option<&str>| ConnectionConfig {
        tls: TlsConfig {
            mode,
            ca_file: ca_file.map(str::to_owned),
        },
        ..base.clone()
    };

    // The test CA is not a public one: without the file the server is not trusted.
    let Err(error) = QdrantDriver.connect(&with(TlsMode::Verify, None)).await else {
        panic!("trusted an unknown CA");
    };
    assert!(matches!(error, Error::Connect(_)), "{error:?}");

    let conn = QdrantDriver
        .connect(&with(TlsMode::Verify, Some(&ca)))
        .await
        .unwrap();
    conn.ping().await.unwrap();
    assert!(conn.vector().unwrap().list_collections().await.is_ok());

    let conn = QdrantDriver
        .connect(&with(TlsMode::Insecure, None))
        .await
        .unwrap();
    conn.ping().await.unwrap();

    // The server speaks TLS only.
    assert!(
        QdrantDriver
            .connect(&with(TlsMode::Off, None))
            .await
            .is_err()
    );

    let missing = with(TlsMode::Verify, Some("/nonexistent/ca.pem"));
    assert!(matches!(
        QdrantDriver.connect(&missing).await,
        Err(Error::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn tls_through_a_route_checks_the_real_host() {
    let Some((base, ca)) = tls_server() else {
        eprintln!("skipped: DBCLIENT_TEST_QDRANT_TLS_URL is not set");
        return;
    };
    let route = format!("{}:{}", base.host.as_deref().unwrap(), base.port.unwrap());
    let through_route = |host: &str| ConnectionConfig {
        host: Some(host.into()),
        port: Some(6333),
        route: Some(route.parse().expect("the test URL needs an IP address")),
        tls: TlsConfig {
            mode: TlsMode::Verify,
            ca_file: Some(ca.clone()),
        },
        ..base.clone()
    };

    // A name and an address the certificate is valid for, reached at the route.
    for host in ["localhost", "192.0.2.10"] {
        let conn = QdrantDriver.connect(&through_route(host)).await.unwrap();
        conn.ping().await.unwrap();
    }
    // The certificate is not valid for these, although the route's own
    // address (127.0.0.1) is in it.
    for host in ["qdrant.invalid", "192.0.2.11"] {
        let Err(error) = QdrantDriver.connect(&through_route(host)).await else {
            panic!("accepted a certificate that does not name {host}");
        };
        assert!(matches!(error, Error::Connect(_)), "{host}: {error:?}");
    }
}
