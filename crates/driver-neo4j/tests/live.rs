//! Tests against a real server. They run only when `DBCLIENT_TEST_NEO4J_URL`
//! is set, for example `http://neo4j:testtest1@127.0.0.1:7474`
//! (`scripts/test-neo4j.sh up` starts a server and prints the line).
//!
//! Every test builds its own small graph under labels no other test uses and
//! deletes it at the end, so the tests can run side by side.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dbclient_core::graph::{CypherRequest, CypherResult, Direction, GraphCell, NeighbourRequest};
use dbclient_core::{
    Connection, ConnectionConfig, DataModel, Driver, Error, ObjectKind, QueryRequest, TlsMode,
    Value,
};
use dbclient_driver_neo4j::Neo4jDriver;
use serde_json::json;

fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_NEO4J_URL").ok()?;
    let url = reqwest::Url::parse(&url).expect("invalid DBCLIENT_TEST_NEO4J_URL");
    Some(ConnectionConfig {
        driver: "neo4j".into(),
        host: url.host_str().map(str::to_owned),
        port: url.port(),
        user: Some(url.username().to_owned()).filter(|u| !u.is_empty()),
        password: url.password().map(str::to_owned),
        database: Some(url.path().trim_matches('/').to_owned()).filter(|d| !d.is_empty()),
        ..ConnectionConfig::default()
    })
}

macro_rules! connect_or_skip {
    () => {
        match test_config() {
            Some(config) => Neo4jDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_NEO4J_URL is not set");
                return;
            }
        }
    };
}

/// A label or type name no other test, and no earlier run, has used.
fn unique(prefix: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}_{nanos}_{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn cypher(statement: &str) -> CypherRequest {
    CypherRequest {
        statement: statement.into(),
        ..CypherRequest::default()
    }
}

async fn run(conn: &dyn Connection, statement: &str) -> CypherResult {
    conn.graph()
        .unwrap()
        .run(cypher(statement))
        .await
        .unwrap()
        .pop()
        .unwrap()
}

async fn delete_label(conn: &dyn Connection, label: &str) {
    run(conn, &format!("MATCH (n:{label}) DETACH DELETE n")).await;
}

fn value(cell: &GraphCell) -> &Value {
    match cell {
        GraphCell::Value { value } => value,
        other => panic!("not a plain value: {other:?}"),
    }
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

#[tokio::test]
async fn connects_and_reports_the_version() {
    let conn = connect_or_skip!();
    let info = Neo4jDriver.info();
    assert_eq!((info.id, info.name), ("neo4j", "Neo4j"));
    assert_eq!(info.model, DataModel::Graph);
    assert_eq!(info.default_port, Some(7474));

    conn.ping().await.unwrap();
    assert!(conn.is_alive().await);
    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.chars().next().unwrap().is_ascii_digit(),
        "{version}"
    );
    eprintln!("server version: {version}");
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("neo4j")
    );
    assert!(!conn.in_transaction().await.unwrap());
    assert_eq!(conn.quote_ident("odd`name"), "`odd``name`");
    assert!(matches!(
        conn.describe_object(None, "Person").await,
        Err(Error::Unsupported(_))
    ));
    conn.close().await.unwrap();
}

#[tokio::test]
async fn returns_scalars_lists_and_maps() {
    let conn = connect_or_skip!();
    let statement = "RETURN 1 AS i, 9223372036854775807 AS big, 1.5 AS f, 'héllo' AS s, \
                     true AS b, null AS n, [1, 'two', null] AS l, {a: 1, b: [2]} AS m, \
                     date('2026-01-02') AS d, datetime('2026-01-02T03:04:05Z') AS dt, \
                     duration('P1D') AS dur, 0.0 / 0.0 AS nan";
    let result = run(&*conn, statement).await;
    assert_eq!(
        result.columns,
        [
            "i", "big", "f", "s", "b", "n", "l", "m", "d", "dt", "dur", "nan"
        ]
    );
    let expected = vec![
        Value::Int(1),
        Value::Int(i64::MAX),
        Value::Float(1.5),
        Value::Text("héllo".into()),
        Value::Bool(true),
        Value::Null,
        Value::Json(json!([1, "two", null])),
        Value::Json(json!({"a": 1, "b": [2]})),
        Value::Temporal("2026-01-02".into()),
        Value::Temporal("2026-01-02T03:04:05Z".into()),
        Value::Text("P1D".into()),
        Value::Text("NaN".into()),
    ];
    let cells: Vec<Value> = result.rows[0].iter().map(|c| value(c).clone()).collect();
    assert_eq!(cells, expected);
    assert_eq!(result.node_count, 0);
    assert!(result.graph.nodes.is_empty());
    assert!(!result.truncated);

    // The query tab gets the same values through the SQL-shaped call.
    let table = conn
        .execute(QueryRequest {
            statement: statement.into(),
            max_rows: None,
        })
        .await
        .unwrap();
    assert_eq!(table.rows, vec![expected]);
    assert_eq!(table.columns[0].name, "i");
    assert_eq!(table.rows_affected, None);
}

#[tokio::test]
async fn returns_nodes_relationships_and_paths() {
    let conn = connect_or_skip!();
    let label = unique("Chain");
    let created = run(
        &*conn,
        &format!(
            "CREATE (a:{label}:Extra {{name: 'a', big: 9007199254740993}})\
             -[:NEXT {{weight: 1}}]->(b:{label} {{name: 'b'}})-[:NEXT]->(c:{label} {{name: 'c'}})"
        ),
    )
    .await;
    let counters = created.counters.unwrap();
    assert_eq!(
        (counters.nodes_created, counters.relationships_created),
        (3, 2)
    );
    assert!(created.columns.is_empty());

    let result = run(
        &*conn,
        &format!(
            "MATCH p = (a:{label} {{name: 'a'}})-[r]->(b)-[]->(c) \
             RETURN a, r, p, [b, c] AS list, {{last: c}} AS map"
        ),
    )
    .await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];

    let GraphCell::Node(a) = &row[0] else {
        panic!("{:?}", row[0]);
    };
    assert_eq!(
        sorted(a.labels.clone()),
        sorted(vec![label.clone(), "Extra".into()])
    );
    assert_eq!(a.properties["name"], json!("a"));
    // Beyond 2^53: sent as text so the interface does not round it.
    assert_eq!(a.properties["big"], json!("9007199254740993"));

    let GraphCell::Relationship(r) = &row[1] else {
        panic!("{:?}", row[1]);
    };
    assert_eq!(r.rel_type, "NEXT");
    assert_eq!(r.start, a.id);
    assert_eq!(r.properties["weight"], json!(1));

    let GraphCell::Path {
        nodes,
        relationships,
    } = &row[2]
    else {
        panic!("{:?}", row[2]);
    };
    let names: Vec<_> = nodes.iter().map(|n| n.properties["name"].clone()).collect();
    assert_eq!(names, [json!("a"), json!("b"), json!("c")]);
    assert_eq!(relationships.len(), 2);
    assert_eq!(relationships[0], *r);
    assert_eq!(relationships[0].end, nodes[1].id);
    assert_eq!(relationships[1].start, nodes[1].id);
    assert_eq!(relationships[1].end, nodes[2].id);

    // Inside a list or a map a node is a JSON object of the same shape.
    let Value::Json(list) = value(&row[3]) else {
        panic!("{:?}", row[3]);
    };
    assert_eq!(list[0]["id"], json!(nodes[1].id));
    assert_eq!(list[1]["properties"]["name"], json!("c"));
    let Value::Json(map) = value(&row[4]) else {
        panic!("{:?}", row[4]);
    };
    assert_eq!(map["last"]["labels"], json!([label]));

    // The graph part has every node and relationship once, although the row
    // mentions them several times.
    assert_eq!(result.graph.nodes, *nodes);
    assert_eq!(result.graph.relationships, *relationships);
    assert_eq!((result.node_count, result.relationship_count), (3, 2));
    assert_eq!((result.nodes_omitted, result.relationships_omitted), (0, 0));

    // Several rows that share nodes.
    let result = run(
        &*conn,
        &format!("MATCH (x:{label})-[r]->(y:{label}) RETURN x, r, y ORDER BY x.name"),
    )
    .await;
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.graph.nodes.len(), 3);
    assert_eq!(result.graph.relationships.len(), 2);

    // Relationships alone: their end nodes are fetched so they can be drawn.
    let result = run(
        &*conn,
        &format!("MATCH (:{label})-[r]->(:{label}) RETURN r"),
    )
    .await;
    assert_eq!(result.graph.relationships.len(), 2);
    assert_eq!(result.graph.nodes.len(), 3);
    assert_eq!((result.node_count, result.nodes_omitted), (3, 0));

    // What the interface receives.
    let wire = serde_json::to_value(&row[0]).unwrap();
    assert_eq!(wire["kind"], json!("node"));
    assert_eq!(wire["id"], json!(a.id));
    assert_eq!(
        serde_json::to_value(&row[1]).unwrap()["type"],
        json!("NEXT")
    );
    assert_eq!(
        serde_json::to_value(&row[2]).unwrap()["kind"],
        json!("path")
    );

    // The plain grid shows a node as JSON.
    let table = conn
        .execute(QueryRequest {
            statement: format!("MATCH (n:{label} {{name: 'b'}}) RETURN n"),
            max_rows: None,
        })
        .await
        .unwrap();
    let Value::Json(node) = &table.rows[0][0] else {
        panic!("{:?}", table.rows[0][0]);
    };
    assert_eq!(node["properties"], json!({"name": "b"}));

    delete_label(&*conn, &label).await;
}

#[tokio::test]
async fn lists_labels_types_and_databases() {
    let conn = connect_or_skip!();
    let graph = conn.graph().unwrap();
    let label = unique("Listed");
    let other = unique("ListedOther");
    let rel_type = unique("LINKS");
    run(
        &*conn,
        &format!(
            "CREATE (a:{label})-[:{rel_type}]->(b:{label}), (a)-[:{rel_type}]->(c:{label}:{other})"
        ),
    )
    .await;

    let labels = graph.labels(None).await.unwrap();
    let count_of = |name: &str| labels.iter().find(|l| l.name == name).map(|l| l.count);
    assert_eq!(count_of(&label), Some(Some(3)));
    assert_eq!(count_of(&other), Some(Some(1)));
    let names: Vec<_> = labels.iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, sorted(names.clone()));

    let types = graph.relationship_types(Some("neo4j")).await.unwrap();
    let listed = types.iter().find(|t| t.name == rel_type).unwrap();
    assert_eq!(listed.count, Some(2));

    let objects = conn.list_objects(None).await.unwrap();
    let object = objects.iter().find(|o| o.name == label).unwrap();
    assert_eq!(object.kind, ObjectKind::Collection);
    assert_eq!(object.row_estimate, Some(3));

    let databases = graph.databases().await.unwrap();
    let neo4j = databases.iter().find(|d| d.name == "neo4j").unwrap();
    assert!(neo4j.is_default && !neo4j.is_system);
    assert_eq!(neo4j.status.as_deref(), Some("online"));
    assert!(databases.iter().any(|d| d.name == "system" && d.is_system));
    let namespaces: Vec<_> = conn
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(namespaces.contains(&"neo4j".to_owned()));
    assert!(!namespaces.contains(&"system".to_owned()));

    delete_label(&*conn, &label).await;
}

#[tokio::test]
async fn expands_neighbours_up_to_a_limit() {
    let conn = connect_or_skip!();
    let graph = conn.graph().unwrap();
    let label = unique("Hub");
    // Five outgoing, two incoming from one node, and one from the hub to itself.
    let created = run(
        &*conn,
        &format!(
            "CREATE (hub:{label} {{name: 'hub'}}), (fan:{label} {{name: 'fan'}}), \
                    (fan)-[:LIKES]->(hub), (fan)-[:FOLLOWS]->(hub), (hub)-[:SELF]->(hub) \
             WITH hub UNWIND range(1, 5) AS i \
             CREATE (hub)-[:OWNS {{n: i}}]->(:{label} {{name: 'leaf' + i}}) \
             RETURN DISTINCT hub"
        ),
    )
    .await;
    let GraphCell::Node(hub) = &created.rows[0][0] else {
        panic!("{:?}", created.rows[0][0]);
    };
    let request = |limit, rel_type: Option<&str>, direction| NeighbourRequest {
        database: None,
        node_id: hub.id.clone(),
        limit,
        rel_type: rel_type.map(str::to_owned),
        direction,
    };

    let some = graph
        .neighbours(request(3, None, Direction::Both))
        .await
        .unwrap();
    assert_eq!(some.relationships.len(), 3);
    assert_eq!(some.total, 8);
    assert!(!some.nodes.is_empty() && some.nodes.len() <= 3);
    for relationship in &some.relationships {
        let other = if relationship.start == hub.id {
            &relationship.end
        } else {
            &relationship.start
        };
        assert!(some.nodes.iter().any(|n| &n.id == other));
    }
    let groups: Vec<_> = some
        .groups
        .iter()
        .map(|g| (g.rel_type.as_str(), g.outgoing, g.count))
        .collect();
    assert_eq!(
        groups,
        [
            ("FOLLOWS", false, 1),
            ("LIKES", false, 1),
            ("OWNS", true, 5),
            ("SELF", true, 1)
        ]
    );

    let all = graph
        .neighbours(request(100, None, Direction::Both))
        .await
        .unwrap();
    assert_eq!(all.relationships.len(), 8);
    // Five leaves, the fan (once, though two relationships lead to it) and the hub itself.
    assert_eq!(all.nodes.len(), 7);

    let owned = graph
        .neighbours(request(2, Some("OWNS"), Direction::Outgoing))
        .await
        .unwrap();
    assert_eq!((owned.relationships.len(), owned.total), (2, 5));
    assert!(owned.relationships.iter().all(|r| r.rel_type == "OWNS"));
    assert_eq!(owned.groups.len(), 4);

    let incoming = graph
        .neighbours(request(100, None, Direction::Incoming))
        .await
        .unwrap();
    // The two from the fan and the hub's own.
    assert_eq!((incoming.relationships.len(), incoming.total), (3, 3));

    let missing = NeighbourRequest {
        node_id: "4:00000000-0000-0000-0000-000000000000:987654321".into(),
        ..request(10, None, Direction::Both)
    };
    assert!(matches!(
        graph.neighbours(missing).await,
        Err(Error::Query(_))
    ));

    delete_label(&*conn, &label).await;
}

#[tokio::test]
async fn caps_the_graph_but_not_the_table() {
    let conn = connect_or_skip!();
    let graph = conn.graph().unwrap();
    let label = unique("Capped");
    // A chain of ten nodes.
    run(
        &*conn,
        &format!(
            "UNWIND range(0, 9) AS i CREATE (:{label} {{i: i}}) \
             WITH count(*) AS made \
             MATCH (a:{label}), (b:{label}) WHERE b.i = a.i + 1 CREATE (a)-[:NEXT]->(b)"
        ),
    )
    .await;

    let request = CypherRequest {
        statement: format!(
            "MATCH (n:{label}) OPTIONAL MATCH (n)-[r:NEXT]->() RETURN n, r ORDER BY n.i"
        ),
        node_limit: Some(4),
        ..CypherRequest::default()
    };
    let result = graph.run(request.clone()).await.unwrap().pop().unwrap();
    assert_eq!(result.rows.len(), 10);
    assert!(matches!(result.rows[9][0], GraphCell::Node(_)));
    assert_eq!(result.graph.nodes.len(), 4);
    assert_eq!((result.node_count, result.nodes_omitted), (10, 6));
    // Only what joins two drawn nodes: 0-1, 1-2, 2-3.
    assert_eq!(result.graph.relationships.len(), 3);
    assert_eq!(
        (result.relationship_count, result.relationships_omitted),
        (9, 6)
    );
    let drawn: Vec<_> = result
        .graph
        .nodes
        .iter()
        .map(|n| n.properties["i"].clone())
        .collect();
    assert_eq!(drawn, [json!(0), json!(1), json!(2), json!(3)]);

    // Without a limit everything is there.
    let all = graph
        .run(CypherRequest {
            node_limit: None,
            ..request.clone()
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!((all.graph.nodes.len(), all.nodes_omitted), (10, 0));
    assert_eq!(all.graph.relationships.len(), 9);

    // Relationships alone with a limit: end nodes are fetched only up to it.
    let alone = graph
        .run(CypherRequest {
            statement: format!("MATCH (:{label})-[r:NEXT]->() RETURN r"),
            node_limit: Some(4),
            ..CypherRequest::default()
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(alone.graph.nodes.len(), 4);
    assert_eq!((alone.node_count, alone.nodes_omitted), (10, 6));
    assert_eq!(alone.relationship_count, 9);

    // The row limit cuts the table, and the graph follows the rows kept.
    let few = graph
        .run(CypherRequest {
            max_rows: Some(2),
            ..request
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(few.truncated);
    assert_eq!(few.rows.len(), 2);
    assert_eq!(few.node_count, 3);

    delete_label(&*conn, &label).await;
}

#[tokio::test]
async fn runs_several_statements_with_parameters() {
    let conn = connect_or_skip!();
    let graph = conn.graph().unwrap();
    let label = unique("Multi");
    let mut request = cypher(&format!(
        "CREATE (:{label} {{name: $name, note: 'a;b'}}); // a comment; with a semicolon\n\
         MATCH (n:{label}) RETURN n.name AS name, n.note AS note;"
    ));
    request.parameters.insert("name".into(), json!("Ada"));
    let results = graph.run(request).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].counters.unwrap().nodes_created, 1);
    assert_eq!(value(&results[1].rows[0][0]), &Value::Text("Ada".into()));
    assert_eq!(value(&results[1].rows[0][1]), &Value::Text("a;b".into()));

    let tables = conn
        .execute_all(QueryRequest {
            statement: format!(
                "MATCH (n:{label}) SET n.seen = true; MATCH (n:{label}) RETURN count(n) AS c"
            ),
            max_rows: None,
        })
        .await
        .unwrap();
    assert_eq!(tables.len(), 2);
    assert!(tables[0].columns.is_empty());
    assert_eq!(tables[0].rows_affected, Some(1));
    assert_eq!(tables[1].rows, vec![vec![Value::Int(1)]]);

    assert!(matches!(
        conn.execute_all(QueryRequest {
            statement: " ; // nothing\n".into(),
            max_rows: None,
        })
        .await,
        Err(Error::Query(_))
    ));

    delete_label(&*conn, &label).await;
}

#[tokio::test]
async fn reports_cypher_errors_with_code_and_position() {
    let conn = connect_or_skip!();
    let graph = conn.graph().unwrap();

    match graph.run(cypher("MATCH (n) RETRUN n")).await {
        Err(Error::Database {
            code,
            message,
            position,
            line,
            ..
        }) => {
            assert_eq!(
                code.as_deref(),
                Some("Neo.ClientError.Statement.SyntaxError")
            );
            assert!(message.contains("RETRUN"), "{message}");
            // The drawing of the place is taken off the message.
            assert!(!message.contains('^'), "{message}");
            assert_eq!(position, Some(11));
            assert_eq!(line, Some(1));
        }
        other => panic!("{other:?}"),
    }

    // In a later statement the position still counts from the start of the text.
    let text = "RETURN 1;\nRETURN 2;\nMATCH (n) RETRUN n";
    match conn
        .execute_all(QueryRequest {
            statement: text.into(),
            max_rows: None,
        })
        .await
    {
        Err(Error::Database { position, line, .. }) => {
            let at = text.find("RETRUN").unwrap() as u32;
            assert_eq!(position, Some(at + 1));
            assert_eq!(line, Some(3));
        }
        other => panic!("{other:?}"),
    }

    // An error while the statement runs has a code but no position.
    match graph.run(cypher("RETURN 1 / 0")).await {
        Err(Error::Database { code, position, .. }) => {
            assert_eq!(
                code.as_deref(),
                Some("Neo.ClientError.Statement.ArithmeticError")
            );
            assert_eq!(position, None);
        }
        other => panic!("{other:?}"),
    }

    // The session is unaffected.
    conn.ping().await.unwrap();
}

#[tokio::test]
async fn rejects_wrong_settings_when_connecting() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_NEO4J_URL is not set");
        return;
    };

    let wrong_password = ConnectionConfig {
        password: Some("not-the-password".into()),
        ..config.clone()
    };
    match Neo4jDriver.connect(&wrong_password).await {
        Err(Error::Connect(message)) => assert!(message.contains("Unauthorized"), "{message}"),
        other => panic!("{:?}", other.err()),
    }

    let no_such_database = ConnectionConfig {
        database: Some("nosuchdatabase".into()),
        ..config.clone()
    };
    match Neo4jDriver.connect(&no_such_database).await {
        Err(Error::Connect(message)) => assert!(message.contains("DatabaseNotFound"), "{message}"),
        other => panic!("{:?}", other.err()),
    }

    // Nothing listens there.
    let closed_port = ConnectionConfig {
        host: Some("127.0.0.1".into()),
        port: Some(1),
        ..config.clone()
    };
    assert!(matches!(
        Neo4jDriver.connect(&closed_port).await,
        Err(Error::Connect(_))
    ));

    // The server does not speak TLS on its plain port.
    let mut encrypted = config;
    encrypted.tls.mode = TlsMode::Insecure;
    assert!(matches!(
        Neo4jDriver.connect(&encrypted).await,
        Err(Error::Connect(_))
    ));
}

#[tokio::test]
async fn follows_a_route_instead_of_the_host() {
    let Some(config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_NEO4J_URL is not set");
        return;
    };
    let real = format!(
        "{}:{}",
        config.host.as_deref().unwrap(),
        config.port.unwrap()
    );
    let route = tokio::net::lookup_host(real).await.unwrap().next();

    // Neither host can be reached: the name does not resolve, the address is
    // reserved for documentation. Only the route leads to the server.
    for host in ["neo4j.invalid", "192.0.2.1"] {
        let routed = ConnectionConfig {
            host: Some(host.into()),
            port: Some(7474),
            route,
            ..config.clone()
        };
        let conn = Neo4jDriver.connect(&routed).await.unwrap();
        let result = run(&*conn, "RETURN 1 AS one").await;
        assert_eq!(value(&result.rows[0][0]), &Value::Int(1));
    }
}
