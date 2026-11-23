//! Tests against a real server. They run only when `DBCLIENT_TEST_MONGODB_URL`
//! is set, for example `mongodb://root:test@127.0.0.1:27017/`; the user in it
//! must be defined in `admin` and allowed to create and drop databases.
//! `scripts/test-mongodb.sh up` starts such a server and prints the variable.
//!
//! The TLS tests also need `DBCLIENT_TEST_MONGODB_CA`, the CA file that signed
//! the server's certificate, which the script prints as well.
//!
//! Every test works in a database of its own, named `dbclient_test_…`, and
//! drops it first, so tests can run in parallel and be repeated.

use std::time::{Duration, Instant};

use dbclient_core::document::{
    CollectionKind, DocumentChange, DocumentStore, FindRequest, type_name,
};
use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, QueryRequest, TlsConfig,
    TlsMode, Value,
};
use dbclient_driver_mongodb::MongoDriver;
use mongodb::options::{ConnectionString, HostInfo, ServerAddress};
use serde_json::json;

/// Settings for the test server, with `database` as the connection's database.
fn test_config(database: Option<&str>) -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_MONGODB_URL").ok()?;
    let parsed = ConnectionString::parse(&url).expect("invalid DBCLIENT_TEST_MONGODB_URL");
    let HostInfo::HostIdentifiers(hosts) = &parsed.host_info else {
        panic!("the test URL needs a host and port");
    };
    let Some(ServerAddress::Tcp { host, port }) = hosts.first() else {
        panic!("the test URL needs a TCP host");
    };
    let credential = parsed.credential.as_ref();
    let mut config = ConnectionConfig {
        driver: "mongodb".into(),
        host: Some(host.clone()),
        port: *port,
        user: credential.and_then(|c| c.username.clone()),
        password: credential.and_then(|c| c.password.clone()),
        database: database.map(str::to_owned),
        ..ConnectionConfig::default()
    };
    // The test user lives in `admin`, not in the database a test works in.
    config.params.insert("authSource".into(), "admin".into());
    Some(config)
}

/// Connects with a fresh, empty database selected.
macro_rules! connect_or_skip {
    ($database:expr) => {
        match test_config(Some($database)) {
            Some(config) => {
                let conn = MongoDriver.connect(&config).await.unwrap();
                run(&*conn, "{ dropDatabase: 1 }").await;
                conn
            }
            None => {
                eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL is not set");
                return;
            }
        }
    };
}

async fn run(conn: &dyn Connection, command: &str) -> Vec<Vec<Value>> {
    conn.execute(QueryRequest {
        statement: command.into(),
        max_rows: None,
    })
    .await
    .unwrap()
    .rows
}

async fn insert(store: &dyn DocumentStore, database: &str, collection: &str, document: &str) {
    store
        .write(
            database,
            collection,
            DocumentChange::Insert {
                document: document.into(),
            },
        )
        .await
        .unwrap();
}

fn request(database: &str, collection: &str) -> FindRequest {
    FindRequest {
        database: database.into(),
        collection: collection.into(),
        limit: 50,
        ..FindRequest::default()
    }
}

/// Six products with whole-number ids 1–6.
async fn seed_products(store: &dyn DocumentStore, database: &str) {
    for (id, name, price, tags) in [
        (1, "anvil", 120.0, "['heavy']"),
        (2, "bolt", 0.5, "['small', 'steel']"),
        (3, "chisel", 14.0, "['steel']"),
        (4, "drill", 89.5, "[]"),
        (5, "epoxy", 7.25, "['small']"),
        (6, "file", 9.0, "['steel']"),
    ] {
        insert(
            store,
            database,
            "products",
            &format!("{{ _id: {id}, name: '{name}', price: {price:?}, tags: {tags} }}"),
        )
        .await;
    }
}

fn ids(page: &dbclient_core::document::FindPage) -> Vec<i64> {
    page.documents
        .iter()
        .map(|d| {
            d.document["_id"]["$numberInt"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn connects_and_reports_the_version() {
    let info = MongoDriver.info();
    assert_eq!((info.id, info.name), ("mongodb", "MongoDB"));
    assert_eq!(info.model, DataModel::Document);
    assert_eq!(info.default_port, Some(27017));

    let conn = connect_or_skip!("dbclient_test_connect");
    conn.ping().await.unwrap();
    assert!(conn.is_alive().await);
    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.split('.').count() >= 3 && version.starts_with(|c: char| c.is_ascii_digit()),
        "{version}"
    );
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("dbclient_test_connect")
    );
    assert!(conn.document().is_some());
    assert!(!conn.in_transaction().await.unwrap());
    assert_eq!(conn.quote_ident("a b"), "a b");
    conn.close().await.unwrap();
}

#[tokio::test]
async fn connects_without_a_database() {
    let Some(mut config) = test_config(None) else {
        eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL is not set");
        return;
    };
    // No database and no authSource: the user is looked up in `admin`.
    config.params.clear();
    let conn = MongoDriver.connect(&config).await.unwrap();
    assert_eq!(conn.current_namespace().await.unwrap(), None);
    let names: Vec<_> = conn
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(names.contains(&"admin".to_owned()), "{names:?}");
    // Without a database there is nothing for an unqualified listing to mean.
    assert!(matches!(
        conn.list_objects(None).await,
        Err(Error::Query(_))
    ));
}

#[tokio::test]
async fn lists_databases_and_collections() {
    let db = "dbclient_test_listing";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();

    for i in 0..10 {
        insert(
            store,
            db,
            "orders",
            &format!("{{ n: {i}, note: 'some text' }}"),
        )
        .await;
    }
    insert(store, db, "customers", "{ name: 'Ada' }").await;
    run(
        &*conn,
        "{ create: 'big_orders', viewOn: 'orders', pipeline: [{ $match: { n: { $gte: 5 } } }] }",
    )
    .await;

    let databases = store.list_databases().await.unwrap();
    let ours = databases
        .iter()
        .find(|d| d.name == db)
        .expect("database listed");
    assert!(ours.size_bytes.is_some());
    assert!(
        conn.list_namespaces()
            .await
            .unwrap()
            .iter()
            .any(|n| n.name == db)
    );

    let collections = store.list_collections(db).await.unwrap();
    let names: Vec<_> = collections
        .iter()
        .map(|c| (c.name.as_str(), c.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("big_orders", CollectionKind::View),
            ("customers", CollectionKind::Collection),
            ("orders", CollectionKind::Collection),
        ]
    );
    let orders = &collections[2];
    assert_eq!(orders.document_count, Some(10));
    let avg = orders.avg_document_bytes.unwrap();
    assert!((30..80).contains(&avg), "{avg}");
    assert_eq!(orders.size_bytes, Some(avg * 10));
    // A view has no stored documents to measure.
    assert_eq!(collections[0].document_count, None);

    assert_eq!(&store.collection_info(db, "orders").await.unwrap(), orders);
    assert!(matches!(
        store.collection_info(db, "missing").await,
        Err(Error::Query(_))
    ));

    // The same listing through the SQL-shaped methods; `None` is the connection's database.
    let objects = conn.list_objects(None).await.unwrap();
    let summary: Vec<_> = objects
        .iter()
        .map(|o| (o.name.as_str(), o.kind, o.row_estimate))
        .collect();
    assert_eq!(
        summary,
        [
            ("big_orders", ObjectKind::View, None),
            ("customers", ObjectKind::Collection, Some(1)),
            ("orders", ObjectKind::Collection, Some(10)),
        ]
    );
    assert_eq!(conn.list_objects(Some(db)).await.unwrap(), objects);
}

#[tokio::test]
async fn finds_with_filter_projection_sort_limit_and_skip() {
    let db = "dbclient_test_find";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    seed_products(store, db).await;

    // Everything, in a stated order.
    let all = store
        .find(FindRequest {
            sort: Some("{ _id: 1 }".into()),
            ..request(db, "products")
        })
        .await
        .unwrap();
    assert_eq!(ids(&all), [1, 2, 3, 4, 5, 6]);
    assert!(!all.has_more);

    // Filter in shell style and in strict JSON.
    for filter in ["{ price: { $lt: 10 } }", r#"{"price": {"$lt": 10}}"#] {
        let cheap = store
            .find(FindRequest {
                filter: Some(filter.into()),
                sort: Some("{ price: -1 }".into()),
                ..request(db, "products")
            })
            .await
            .unwrap();
        assert_eq!(ids(&cheap), [6, 5, 2], "{filter}");
    }

    // A filter on an array field, and a blank filter meaning none.
    let steel = store
        .find(FindRequest {
            filter: Some("{ tags: 'steel', price: { $gte: 9 } }".into()),
            sort: Some("{ _id: 1 }".into()),
            ..request(db, "products")
        })
        .await
        .unwrap();
    assert_eq!(ids(&steel), [3, 6]);
    let blank = store
        .find(FindRequest {
            filter: Some("  ".into()),
            ..request(db, "products")
        })
        .await
        .unwrap();
    assert_eq!(blank.documents.len(), 6);

    // Projection: only the named field, and _id unless it is switched off.
    let names = store
        .find(FindRequest {
            projection: Some("{ name: 1, _id: 0 }".into()),
            sort: Some("{ name: -1 }".into()),
            limit: 2,
            ..request(db, "products")
        })
        .await
        .unwrap();
    let documents: Vec<_> = names.documents.iter().map(|d| d.document.clone()).collect();
    assert_eq!(
        documents,
        [json!({"name": "file"}), json!({"name": "epoxy"})]
    );
    assert!(names.has_more);

    // Pages: limit and skip, with has_more false exactly on the last one.
    let mut seen = Vec::new();
    for (skip, expected_more) in [(0, true), (4, false)] {
        let page = store
            .find(FindRequest {
                sort: Some("{ price: 1 }".into()),
                skip,
                limit: 4,
                ..request(db, "products")
            })
            .await
            .unwrap();
        assert_eq!(page.skip, skip);
        assert_eq!(page.has_more, expected_more);
        seen.extend(ids(&page));
    }
    assert_eq!(seen, [2, 5, 6, 3, 4, 1]);
    let exact = store
        .find(FindRequest {
            limit: 6,
            ..request(db, "products")
        })
        .await
        .unwrap();
    assert_eq!((exact.documents.len(), exact.has_more), (6, false));

    // A collection that does not exist is simply empty, as in MongoDB itself.
    let none = store.find(request(db, "missing")).await.unwrap();
    assert!(none.documents.is_empty() && !none.has_more);
}

#[tokio::test]
async fn reports_the_type_of_every_value() {
    let db = "dbclient_test_types";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();

    insert(
        store,
        db,
        "things",
        r#"{
            _id: ObjectId("65f1c0a2b7e4d3a1c0ffee01"),
            name: "héllo",
            price: 12.5,
            whole_double: {"$numberDouble": "3.0"},
            stock: 42,
            views: NumberLong("9007199254740993"),
            big: 3000000000,
            amount: NumberDecimal("12.50"),
            created: ISODate("2023-11-14T22:13:20Z"),
            active: true,
            nothing: null,
            size: { w: 2, h: 3 },
            tags: ["a", "b", "c"],
            raw: {"$binary": {"base64": "AP8=", "subType": "00"}},
            pattern: {"$regularExpression": {"pattern": "^a", "options": "i"}},
            stamp: {"$timestamp": {"t": 1700000000, "i": 1}}
        }"#,
    )
    .await;

    let page = store.find(request(db, "things")).await.unwrap();
    let found = &page.documents[0];
    let document = &found.document;

    // The whole shape, once, as the interface receives it. Field order is the stored order.
    assert_eq!(
        serde_json::to_string(document).unwrap(),
        concat!(
            r#"{"_id":{"$oid":"65f1c0a2b7e4d3a1c0ffee01"},"name":"héllo","#,
            r#""price":{"$numberDouble":"12.5"},"whole_double":{"$numberDouble":"3.0"},"#,
            r#""stock":{"$numberInt":"42"},"views":{"$numberLong":"9007199254740993"},"#,
            r#""big":{"$numberLong":"3000000000"},"amount":{"$numberDecimal":"12.50"},"#,
            r#""created":{"$date":{"$numberLong":"1700000000000"}},"active":true,"nothing":null,"#,
            r#""size":{"w":{"$numberInt":"2"},"h":{"$numberInt":"3"}},"tags":["a","b","c"],"#,
            r#""raw":{"$binary":{"base64":"AP8=","subType":"00"}},"#,
            r#""pattern":{"$regularExpression":{"pattern":"^a","options":"i"}},"#,
            r#""stamp":{"$timestamp":{"t":1700000000,"i":1}}}"#
        )
    );

    let types: Vec<_> = document
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.as_str(), type_name(value)))
        .collect();
    assert_eq!(
        types,
        [
            ("_id", "objectId"),
            ("name", "string"),
            ("price", "double"),
            ("whole_double", "double"),
            ("stock", "int32"),
            ("views", "int64"),
            ("big", "int64"),
            ("amount", "decimal128"),
            ("created", "date"),
            ("active", "bool"),
            ("nothing", "null"),
            ("size", "object"),
            ("tags", "array"),
            ("raw", "binData"),
            ("pattern", "regex"),
            ("stamp", "timestamp"),
        ]
    );
    // What the tree shows beside containers: `object {2}`, `array [3]`.
    assert_eq!(document["size"].as_object().unwrap().len(), 2);
    assert_eq!(document["tags"].as_array().unwrap().len(), 3);
    assert_eq!(type_name(&document["size"]["w"]), "int32");
    assert!(found.size_bytes > 200, "{}", found.size_bytes);

    // The server agrees about the types: its own $type operator finds the document.
    for filter in [
        "{ stock: { $type: 'int' }, views: { $type: 'long' }, price: { $type: 'double' } }",
        "{ whole_double: { $type: 'double' }, big: { $type: 'long' }, created: { $type: 'date' } }",
        "{ amount: { $type: 'decimal' }, _id: { $type: 'objectId' }, raw: { $type: 'binData' } }",
    ] {
        assert_eq!(
            store.count(db, "things", Some(filter)).await.unwrap(),
            1,
            "{filter}"
        );
    }

    // Sending a document back as it was received keeps every type.
    let text = serde_json::to_string(document).unwrap();
    let outcome = store
        .write(
            db,
            "things",
            DocumentChange::Replace {
                id: document["_id"].clone(),
                document: text,
            },
        )
        .await
        .unwrap();
    // Matched but not modified: the server saw identical content.
    assert_eq!((outcome.matched, outcome.modified), (1, 0));

    // Typed values in a filter: by ObjectId and by date.
    for filter in [
        r#"{ _id: ObjectId("65f1c0a2b7e4d3a1c0ffee01") }"#,
        r#"{ "_id": {"$oid": "65f1c0a2b7e4d3a1c0ffee01"} }"#,
        r#"{ created: { $gte: ISODate("2023-11-14"), $lt: ISODate("2023-11-15") } }"#,
    ] {
        assert_eq!(
            store.count(db, "things", Some(filter)).await.unwrap(),
            1,
            "{filter}"
        );
    }
    // The same hex digits as text are a different value.
    let as_text = r#"{ _id: "65f1c0a2b7e4d3a1c0ffee01" }"#;
    assert_eq!(store.count(db, "things", Some(as_text)).await.unwrap(), 0);
}

#[tokio::test]
async fn counts_matching_documents() {
    let db = "dbclient_test_count";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    seed_products(store, db).await;

    assert_eq!(store.count(db, "products", None).await.unwrap(), 6);
    assert_eq!(store.count(db, "products", Some("")).await.unwrap(), 6);
    assert_eq!(
        store
            .count(db, "products", Some("{ price: { $lt: 10 } }"))
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .count(db, "products", Some("{ name: 'nothing' }"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(store.count(db, "missing", None).await.unwrap(), 0);
}

#[tokio::test]
async fn inserts_replaces_updates_and_deletes_by_id() {
    let db = "dbclient_test_write";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    let write = |change| store.write(db, "notes", change);
    let fetch = || async {
        let page = store.find(request(db, "notes")).await.unwrap();
        page.documents
            .into_iter()
            .map(|d| d.document)
            .collect::<Vec<_>>()
    };

    // The server names a document that comes without an _id.
    let inserted = write(DocumentChange::Insert {
        document: "{ title: 'first', stars: 1 }".into(),
    })
    .await
    .unwrap();
    let id = inserted.inserted_id.clone().unwrap();
    assert_eq!(type_name(&id), "objectId");
    assert_eq!(
        fetch().await,
        [json!({"_id": id, "title": "first", "stars": {"$numberInt": "1"}})]
    );

    // Replace: every field but _id is exchanged; the document may leave _id out.
    let replaced = write(DocumentChange::Replace {
        id: id.clone(),
        document: "{ title: 'second', done: true }".into(),
    })
    .await
    .unwrap();
    assert_eq!((replaced.matched, replaced.modified), (1, 1));
    assert_eq!(
        fetch().await,
        [json!({"_id": id, "title": "second", "done": true})]
    );

    // An _id cannot be changed by a replace (server code 66, ImmutableField).
    let moved = write(DocumentChange::Replace {
        id: id.clone(),
        document: "{ _id: 5, title: 'third' }".into(),
    })
    .await;
    assert!(
        matches!(&moved, Err(Error::Database { code: Some(code), .. }) if code.starts_with("66")),
        "{moved:?}"
    );

    // Update: only the named parts change.
    let updated = write(DocumentChange::Update {
        id: id.clone(),
        update: "{ $set: { stars: NumberLong(5) }, $unset: { done: '' } }".into(),
    })
    .await
    .unwrap();
    assert_eq!((updated.matched, updated.modified), (1, 1));
    assert_eq!(
        fetch().await,
        [json!({"_id": id, "title": "second", "stars": {"$numberLong": "5"}})]
    );

    // An _id that is not there: reported as nothing matched, not as an error.
    let other = json!({"$oid": "000000000000000000000001"});
    let missed = write(DocumentChange::Replace {
        id: other.clone(),
        document: "{ title: 'ghost' }".into(),
    })
    .await
    .unwrap();
    assert_eq!((missed.matched, missed.modified), (0, 0));
    assert_eq!(
        write(DocumentChange::Delete { id: other })
            .await
            .unwrap()
            .deleted,
        0
    );
    assert_eq!(fetch().await.len(), 1);

    // Keys of other types: a number and a string are different keys.
    write(DocumentChange::Insert {
        document: "{ _id: 7, kind: 'number' }".into(),
    })
    .await
    .unwrap();
    let text_key = write(DocumentChange::Insert {
        document: "{ _id: '7', kind: 'text' }".into(),
    })
    .await
    .unwrap();
    assert_eq!(text_key.inserted_id, Some(json!("7")));
    let deleted = write(DocumentChange::Delete {
        id: json!({"$numberInt": "7"}),
    })
    .await
    .unwrap();
    assert_eq!(deleted.deleted, 1);
    assert_eq!(
        store
            .count(db, "notes", Some("{ kind: 'text' }"))
            .await
            .unwrap(),
        1
    );

    // The same _id twice is refused by the server, with its code (DuplicateKey).
    let again = write(DocumentChange::Insert {
        document: "{ _id: '7' }".into(),
    })
    .await;
    assert!(
        matches!(&again, Err(Error::Database { code: Some(code), .. }) if code == "11000"),
        "{again:?}"
    );

    // Delete by _id.
    assert_eq!(
        write(DocumentChange::Delete { id: id.clone() })
            .await
            .unwrap()
            .deleted,
        1
    );
    assert_eq!(
        write(DocumentChange::Delete { id }).await.unwrap().deleted,
        0
    );
    assert_eq!(store.count(db, "notes", None).await.unwrap(), 1);

    // Text that is not a document never reaches the server.
    for document in ["", "{ a: }", "[1, 2]"] {
        let refused = write(DocumentChange::Insert {
            document: document.into(),
        })
        .await;
        assert!(
            matches!(&refused, Err(Error::Database { code: Some(code), .. }) if code == "invalid_document"),
            "{document}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn an_invalid_filter_says_what_is_wrong_and_where() {
    let db = "dbclient_test_invalid";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    seed_products(store, db).await;

    // The text from the design: nothing after `$lt:`.
    let text = r#"{ "category": "tools", "price": { "$gt": 5, "$lt": } }"#;
    let error = store
        .find(FindRequest {
            filter: Some(text.into()),
            ..request(db, "products")
        })
        .await
        .unwrap_err();
    match &error {
        Error::Database {
            code,
            message,
            position,
            ..
        } => {
            assert_eq!(code.as_deref(), Some("invalid_filter"));
            assert_eq!(message, "a value is expected after $lt:");
            // 1-based, pointing at the `}` that stands where the value should.
            assert_eq!(*position, Some(52));
            assert_eq!(text.chars().nth(51), Some('}'));
        }
        other => panic!("{other:?}"),
    }
    // As it crosses to the interface.
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({"kind": "database", "message": {
            "code": "invalid_filter", "message": "a value is expected after $lt:",
            "position": 52, "line": null, "hint": null, "detail": null,
        }})
    );

    // Each field is named in its own error, and count reads filters the same way.
    let code_of = |result: Result<_, Error>| match result {
        Err(Error::Database { code, position, .. }) => (code.unwrap(), position),
        other => panic!("{:?}", other.map(|_: dbclient_core::document::FindPage| ())),
    };
    let projection = store
        .find(FindRequest {
            projection: Some("{ name: 1".into()),
            ..request(db, "products")
        })
        .await;
    assert_eq!(code_of(projection), ("invalid_projection".into(), Some(10)));
    let sort = store
        .find(FindRequest {
            sort: Some("name".into()),
            ..request(db, "products")
        })
        .await;
    assert_eq!(code_of(sort), ("invalid_sort".into(), Some(1)));
    assert!(matches!(
        store.count(db, "products", Some("{ price: { $lt: } }")).await,
        Err(Error::Database { code: Some(code), position: Some(17), .. }) if code == "invalid_filter"
    ));

    // Well-formed text the server does not accept: its message, its code, no position.
    let unknown = store
        .find(FindRequest {
            filter: Some("{ price: { $below: 5 } }".into()),
            ..request(db, "products")
        })
        .await
        .unwrap_err();
    match unknown {
        Error::Database {
            code,
            message,
            position,
            ..
        } => {
            assert_eq!(code.as_deref(), Some("2 (BadValue)"));
            assert!(message.contains("$below"), "{message}");
            assert_eq!(position, None);
        }
        other => panic!("{other:?}"),
    }

    // The connection is fine afterwards.
    assert_eq!(store.count(db, "products", None).await.unwrap(), 6);
}

#[tokio::test]
async fn a_wrong_password_is_a_connect_error() {
    let Some(mut config) = test_config(None) else {
        eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL is not set");
        return;
    };
    config.password = Some("not-the-password".into());
    let started = Instant::now();
    let result = MongoDriver.connect(&config).await;
    match result {
        Err(Error::Connect(message)) => {
            assert!(message.to_lowercase().contains("auth"), "{message}");
            assert!(!message.contains("not-the-password"), "{message}");
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("connected with a wrong password"),
    }
    // Refused by the server, not found out by waiting for a timeout.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );

    // An unknown user and a user looked up in the wrong database fail the same way.
    let mut config = test_config(None).unwrap();
    config
        .params
        .insert("authSource".into(), "dbclient_test_no_users".into());
    assert!(matches!(
        MongoDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn an_unreachable_server_is_a_connect_error() {
    if test_config(None).is_none() {
        eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL is not set");
        return;
    }
    // A port nothing listens on.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let config = ConnectionConfig {
        driver: "mongodb".into(),
        host: Some("127.0.0.1".into()),
        port: Some(port),
        ..ConnectionConfig::default()
    };
    assert!(matches!(
        MongoDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn describes_a_collection_from_a_sample() {
    let db = "dbclient_test_describe";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    insert(store, db, "people", "{ _id: 1, name: 'Ada', age: 36 }").await;
    insert(
        store,
        db,
        "people",
        "{ _id: 2, name: 'Bob', age: null, email: 'bob@example.com' }",
    )
    .await;
    insert(store, db, "people", "{ _id: 3, name: 'Cy', age: 41.5 }").await;

    let description = conn.describe_object(None, "people").await.unwrap();
    let columns: Vec<_> = description
        .columns
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.type_name.as_deref().unwrap(),
                c.nullable,
                c.primary_key,
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("_id", "int32", false, true),
            ("name", "string", false, false),
            ("age", "int32 | null | double", true, false),
            ("email", "string", true, false),
        ]
    );
    let structure = conn.table_structure(Some(db), "people").await.unwrap();
    assert_eq!(structure.columns, description.columns);

    assert!(
        conn.describe_object(None, "missing")
            .await
            .unwrap()
            .columns
            .is_empty()
    );
}

#[tokio::test]
async fn runs_a_command_document_and_refuses_sql_shaped_calls() {
    let db = "dbclient_test_command";
    let conn = connect_or_skip!(db);
    let store = conn.document().unwrap();
    seed_products(store, db).await;

    // A reply with a cursor: one row per document.
    let rows = run(
        &*conn,
        "{ find: 'products', filter: { _id: { $lte: 2 } }, sort: { _id: 1 } }",
    )
    .await;
    assert_eq!(rows.len(), 2);
    assert!(matches!(&rows[0][0], Value::Json(d) if d["name"] == "anvil"));

    // Any other reply: one row.
    let rows = run(&*conn, "{ count: 'products' }").await;
    assert!(matches!(&rows[0][0], Value::Json(d) if d["n"] == json!({"$numberInt": "6"})));

    // max_rows, and a first batch that is not the whole answer.
    let limited = conn
        .execute(QueryRequest {
            statement: "{ find: 'products' }".into(),
            max_rows: Some(4),
        })
        .await
        .unwrap();
    assert_eq!((limited.rows.len(), limited.truncated), (4, true));
    let batch = conn
        .execute(QueryRequest {
            statement: "{ find: 'products', batchSize: 2 }".into(),
            max_rows: None,
        })
        .await
        .unwrap();
    assert_eq!((batch.rows.len(), batch.truncated), (2, true));

    // SQL is not a command document, and the server's own refusals carry its code.
    let sql = conn
        .execute(QueryRequest {
            statement: "SELECT 1".into(),
            max_rows: None,
        })
        .await;
    assert!(
        matches!(&sql, Err(Error::Database { code: Some(code), position: Some(1), .. }) if code == "invalid_command"),
        "{sql:?}"
    );
    let unknown = conn
        .execute(QueryRequest {
            statement: "{ noSuchCommand: 1 }".into(),
            max_rows: None,
        })
        .await;
    assert!(
        matches!(&unknown, Err(Error::Database { code: Some(code), .. }) if code == "59 (CommandNotFound)"),
        "{unknown:?}"
    );

    assert!(matches!(
        conn.count_rows(None, "products", None).await,
        Err(Error::Unsupported(_))
    ));
    let page = conn
        .fetch_rows(serde_json::from_value(json!({"table": "products", "limit": 10})).unwrap())
        .await;
    assert!(matches!(page, Err(Error::Unsupported(_))));
    let changes: dbclient_core::ChangeSet =
        serde_json::from_value(json!({"table": "products", "changes": []})).unwrap();
    assert!(matches!(
        conn.apply_changes(&changes).await,
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        conn.preview_changes(&changes).await,
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(conn.cancel().await, Err(Error::Unsupported(_))));
}

#[tokio::test]
async fn a_route_replaces_the_address_but_not_the_name() {
    let Some(mut config) = test_config(Some("dbclient_test_route")) else {
        eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL is not set");
        return;
    };
    // As the tunnel layer does it: the real name stays in `host`, the socket goes to `route`.
    let address = format!(
        "{}:{}",
        config.host.as_deref().unwrap(),
        config.port.unwrap()
    );
    config.route = Some(
        address
            .parse()
            .expect("the test URL needs an IP address for this test"),
    );
    // A name that resolves nowhere, and a port nothing listens on.
    config.host = Some("mongo.dbclient.test".into());
    config.port = Some(1);

    let conn = MongoDriver.connect(&config).await.unwrap();
    let store = conn.document().unwrap();
    run(&*conn, "{ dropDatabase: 1 }").await;
    insert(store, "dbclient_test_route", "c", "{ through: 'route' }").await;
    assert_eq!(
        store.count("dbclient_test_route", "c", None).await.unwrap(),
        1
    );
    conn.close().await.unwrap();

    // A route that leads nowhere fails as a connect error.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    config.route = Some(listener.local_addr().unwrap());
    drop(listener);
    assert!(matches!(
        MongoDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn tls_modes() {
    let (Some(config), Ok(ca)) = (
        test_config(Some("dbclient_test_tls")),
        std::env::var("DBCLIENT_TEST_MONGODB_CA"),
    ) else {
        eprintln!("skipped: DBCLIENT_TEST_MONGODB_URL or DBCLIENT_TEST_MONGODB_CA is not set");
        return;
    };
    let with = |mode, ca_file: Option<&str>| ConnectionConfig {
        tls: TlsConfig {
            mode,
            ca_file: ca_file.map(str::to_owned),
        },
        ..config.clone()
    };
    let works = |config: ConnectionConfig| async move {
        let conn = MongoDriver.connect(&config).await?;
        conn.ping().await?;
        conn.close().await
    };

    // Verified against the test CA.
    works(with(TlsMode::Verify, Some(&ca))).await.unwrap();
    // Without it the certificate is signed by nobody the client trusts.
    let untrusted = works(with(TlsMode::Verify, None)).await;
    assert!(matches!(untrusted, Err(Error::Connect(_))), "{untrusted:?}");
    // Insecure accepts it anyway.
    works(with(TlsMode::Insecure, None)).await.unwrap();
    // A CA file that is not there is a settings problem.
    let missing = works(with(TlsMode::Verify, Some("/no/such/ca.pem"))).await;
    assert!(
        matches!(missing, Err(Error::InvalidConfig(_) | Error::Connect(_))),
        "{missing:?}"
    );

    // Through a route the certificate is checked against `host`, not the route's address.
    let address = format!(
        "{}:{}",
        config.host.as_deref().unwrap(),
        config.port.unwrap()
    );
    let routed = |host: &str| ConnectionConfig {
        host: Some(host.into()),
        port: Some(1),
        route: Some(address.parse().unwrap()),
        ..with(TlsMode::Verify, Some(&ca))
    };
    // The certificate names mongo.dbclient.test …
    works(routed("mongo.dbclient.test")).await.unwrap();
    // … and not this one, although both arrive at the same server.
    let wrong_name = works(routed("other.dbclient.test")).await;
    assert!(
        matches!(wrong_name, Err(Error::Connect(_))),
        "{wrong_name:?}"
    );
}
