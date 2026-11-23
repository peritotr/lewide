//! Tests against a real server. They run only when `DBCLIENT_TEST_REDIS_URL`
//! is set, for example `redis://:test@127.0.0.1:6379/0` (see scripts/test-redis.sh).

use dbclient_core::kv::{KeyCommand, KeyData, ReadRange};
use dbclient_core::{Connection, ConnectionConfig, Driver, Error, QueryRequest, Value};
use dbclient_driver_redis::RedisDriver;

/// `redis://[user]:password@host:port/db`
fn test_config() -> Option<ConnectionConfig> {
    let url = std::env::var("DBCLIENT_TEST_REDIS_URL").ok()?;
    let rest = url
        .strip_prefix("redis://")
        .expect("the test URL starts with redis://");
    let (auth, address) = rest.rsplit_once('@').unwrap_or(("", rest));
    let (user, password) = auth.split_once(':').unwrap_or(("", auth));
    let (host_port, db) = address.split_once('/').unwrap_or((address, "0"));
    let (host, port) = host_port
        .rsplit_once(':')
        .expect("the test URL needs a port");
    Some(ConnectionConfig {
        driver: "redis".into(),
        host: Some(host.into()),
        port: Some(port.parse().unwrap()),
        user: Some(user.to_owned()).filter(|u| !u.is_empty()),
        password: Some(password.to_owned()).filter(|p| !p.is_empty()),
        database: Some(db.into()),
        ..ConnectionConfig::default()
    })
}

macro_rules! connect_or_skip {
    () => {
        match test_config() {
            Some(config) => RedisDriver.connect(&config).await.unwrap(),
            None => {
                eprintln!("skipped: DBCLIENT_TEST_REDIS_URL is not set");
                return;
            }
        }
    };
}

/// Each test works in its own database, so they can run side by side.
const DB_TYPES: u32 = 11;
const DB_SCAN: u32 = 12;
const DB_WRITE: u32 = 13;

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

async fn run(conn: &dyn Connection, line: &str) -> Value {
    conn.execute(QueryRequest {
        statement: line.into(),
        max_rows: None,
    })
    .await
    .unwrap()
    .rows
    .remove(0)
    .remove(0)
}

#[tokio::test]
async fn connects_and_answers_commands_as_text() {
    let conn = connect_or_skip!();
    conn.ping().await.unwrap();
    let version = conn.server_version().await.unwrap().unwrap();
    assert!(
        version.chars().next().unwrap().is_ascii_digit(),
        "{version}"
    );
    assert_eq!(
        conn.current_namespace().await.unwrap().as_deref(),
        Some("db0")
    );
    assert!(
        conn.list_namespaces()
            .await
            .unwrap()
            .iter()
            .any(|n| n.name == "db0")
    );

    assert_eq!(
        run(conn.as_ref(), "SET console:greeting \"hello there\"").await,
        text("OK")
    );
    assert_eq!(
        run(conn.as_ref(), "GET console:greeting").await,
        text("hello there")
    );
    assert_eq!(
        run(conn.as_ref(), "DEL console:greeting").await,
        Value::Int(1)
    );
    // Several lines are several commands; an array reply is one row per item.
    let results = conn
        .execute_all(QueryRequest {
            statement:
                "RPUSH console:list a b c\nLRANGE console:list 0 -1\n# a comment\nDEL console:list;"
                    .into(),
            max_rows: None,
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(
        results[1].rows,
        vec![vec![text("a")], vec![text("b")], vec![text("c")]]
    );

    match conn
        .execute(QueryRequest {
            statement: "NOSUCHCOMMAND x".into(),
            max_rows: None,
        })
        .await
        .unwrap_err()
    {
        Error::Database { code, message, .. } => {
            assert_eq!(code.as_deref(), Some("ERR"));
            assert!(
                message.to_lowercase().contains("unknown command"),
                "{message}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
    // The connection is still usable after an error reply.
    conn.ping().await.unwrap();
    conn.close().await.unwrap();
}

#[tokio::test]
async fn a_wrong_password_is_a_connect_error() {
    let Some(mut config) = test_config() else {
        eprintln!("skipped: DBCLIENT_TEST_REDIS_URL is not set");
        return;
    };
    config.password = Some("not the password".into());
    assert!(matches!(
        RedisDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
    config.password = None;
    assert!(matches!(
        RedisDriver.connect(&config).await,
        Err(Error::Connect(_))
    ));
    config.database = Some("first".into());
    assert!(matches!(
        RedisDriver.connect(&config).await,
        Err(Error::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn reads_every_type() {
    let conn = connect_or_skip!();
    let kv = conn.key_value().expect("Redis is a key-value store");
    let db = DB_TYPES;
    for (key, _) in [
        ("t:string", ()),
        ("t:hash", ()),
        ("t:list", ()),
        ("t:set", ()),
        ("t:zset", ()),
        ("t:expiring", ()),
    ] {
        let _ = kv.write(db, KeyCommand::Delete { key: key.into() }).await;
    }
    kv.write(
        db,
        KeyCommand::SetString {
            key: "t:string".into(),
            value: "plain text".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::HashSet {
            key: "t:hash".into(),
            field: "name".into(),
            value: "Ada".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::HashSet {
            key: "t:hash".into(),
            field: "country".into(),
            value: "GB".into(),
        },
    )
    .await
    .unwrap();
    for item in ["one", "two", "three", "four", "five"] {
        kv.write(
            db,
            KeyCommand::ListPush {
                key: "t:list".into(),
                value: item.into(),
                left: false,
            },
        )
        .await
        .unwrap();
    }
    for member in ["red", "green", "blue"] {
        kv.write(
            db,
            KeyCommand::SetAdd {
                key: "t:set".into(),
                member: member.into(),
            },
        )
        .await
        .unwrap();
    }
    for (member, score) in [("low", 1.0), ("mid", 2.5), ("high", 10.0)] {
        kv.write(
            db,
            KeyCommand::ZsetAdd {
                key: "t:zset".into(),
                member: member.into(),
                score,
            },
        )
        .await
        .unwrap();
    }

    let string = kv.read(db, "t:string", ReadRange::default()).await.unwrap();
    assert_eq!((string.kind.as_str(), string.ttl_ms), ("string", None));
    assert_eq!(
        string.data,
        KeyData::String {
            value: text("plain text")
        }
    );
    assert!(
        string.size_bytes.is_some() && string.encoding.is_some(),
        "{string:?}"
    );

    let KeyData::Hash { mut fields, total } = kv
        .read(db, "t:hash", ReadRange::default())
        .await
        .unwrap()
        .data
    else {
        panic!("not a hash")
    };
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(total, 2);
    assert_eq!(
        fields,
        vec![
            ("country".to_owned(), text("GB")),
            ("name".to_owned(), text("Ada"))
        ]
    );

    let list = kv
        .read(
            db,
            "t:list",
            ReadRange {
                offset: 1,
                limit: 2,
                ..ReadRange::default()
            },
        )
        .await
        .unwrap()
        .data;
    assert_eq!(
        list,
        KeyData::List {
            items: vec![text("two"), text("three")],
            start: 1,
            total: 5
        }
    );

    let KeyData::Set {
        mut members,
        cursor,
        total,
    } = kv
        .read(db, "t:set", ReadRange::default())
        .await
        .unwrap()
        .data
    else {
        panic!("not a set")
    };
    members.sort_by_key(|m| format!("{m:?}"));
    assert_eq!(
        (members, cursor.as_str(), total),
        (vec![text("blue"), text("green"), text("red")], "0", 3)
    );

    let zset = kv
        .read(
            db,
            "t:zset",
            ReadRange {
                descending: true,
                limit: 2,
                ..ReadRange::default()
            },
        )
        .await
        .unwrap()
        .data;
    assert_eq!(
        zset,
        KeyData::Zset {
            members: vec![(text("high"), 10.0), (text("mid"), 2.5)],
            start: 0,
            total: 3
        }
    );
    let zset = kv
        .read(db, "t:zset", ReadRange::default())
        .await
        .unwrap()
        .data;
    assert!(matches!(&zset, KeyData::Zset { members, .. } if members[0] == (text("low"), 1.0)));

    assert!(
        kv.read(db, "t:missing", ReadRange::default())
            .await
            .is_err()
    );
    for key in ["t:string", "t:hash", "t:list", "t:set", "t:zset"] {
        kv.write(db, KeyCommand::Delete { key: key.into() })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn scans_by_pattern_and_type() {
    let conn = connect_or_skip!();
    let kv = conn.key_value().unwrap();
    let db = DB_SCAN;
    for i in 0..250 {
        kv.write(
            db,
            KeyCommand::SetString {
                key: format!("scan:user:{i}"),
                value: "x".into(),
            },
        )
        .await
        .unwrap();
    }
    kv.write(
        db,
        KeyCommand::HashSet {
            key: "scan:cart:1".into(),
            field: "a".into(),
            value: "1".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::Expire {
            key: "scan:cart:1".into(),
            seconds: 600,
        },
    )
    .await
    .unwrap();

    // The scan comes in steps; following the cursor finds every key once.
    let mut cursor = "0".to_owned();
    let mut found = std::collections::BTreeSet::new();
    let mut steps = 0;
    loop {
        let page = kv.scan(db, "scan:user:*", None, &cursor, 50).await.unwrap();
        for key in page.keys {
            assert_eq!(key.kind, "string");
            assert_eq!(key.ttl_ms, None);
            found.insert(key.key);
        }
        steps += 1;
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(found.len(), 250);
    assert!(steps > 1, "250 keys with COUNT 50 take more than one step");

    let mut hashes = Vec::new();
    let mut cursor = "0".to_owned();
    loop {
        let page = kv.scan(db, "", Some("hash"), &cursor, 1000).await.unwrap();
        hashes.extend(page.keys);
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(hashes.len(), 1);
    assert_eq!(hashes[0].key, "scan:cart:1");
    let ttl = hashes[0].ttl_ms.unwrap();
    assert!(ttl > 590_000 && ttl <= 600_000, "{ttl}");

    let info = kv.keyspace(db).await.unwrap();
    assert_eq!(info.keys, 251);
    assert!(info.memory_bytes.unwrap() > 0);
    assert!(info.databases.contains(&(db, 251)), "{:?}", info.databases);

    for key in found {
        kv.write(db, KeyCommand::Delete { key }).await.unwrap();
    }
    kv.write(
        db,
        KeyCommand::Delete {
            key: "scan:cart:1".into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn writes_each_kind_of_change() {
    let conn = connect_or_skip!();
    let kv = conn.key_value().unwrap();
    let db = DB_WRITE;
    let key = |name: &str| format!("w:{name}");
    for name in ["s", "h", "l", "set", "z", "renamed", "other"] {
        let _ = kv.write(db, KeyCommand::Delete { key: key(name) }).await;
    }

    // A string keeps its time to live when its value is replaced.
    kv.write(
        db,
        KeyCommand::SetString {
            key: key("s"),
            value: "one".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::Expire {
            key: key("s"),
            seconds: 300,
        },
    )
    .await
    .unwrap();
    let shown = kv
        .write(
            db,
            KeyCommand::SetString {
                key: key("s"),
                value: "two words".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(shown, "SET w:s \"two words\" KEEPTTL");
    let read = kv.read(db, &key("s"), ReadRange::default()).await.unwrap();
    assert_eq!(
        read.data,
        KeyData::String {
            value: text("two words")
        }
    );
    assert!(read.ttl_ms.unwrap() > 290_000);
    kv.write(db, KeyCommand::Persist { key: key("s") })
        .await
        .unwrap();
    assert_eq!(
        kv.read(db, &key("s"), ReadRange::default())
            .await
            .unwrap()
            .ttl_ms,
        None
    );

    kv.write(
        db,
        KeyCommand::HashSet {
            key: key("h"),
            field: "a".into(),
            value: "1".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::HashSet {
            key: key("h"),
            field: "b".into(),
            value: "2".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::HashDelete {
            key: key("h"),
            field: "a".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        kv.read(db, &key("h"), ReadRange::default())
            .await
            .unwrap()
            .data,
        KeyData::Hash {
            fields: vec![("b".into(), text("2"))],
            total: 1
        }
    );

    // Removing by position takes out that item, not another with the same value.
    for item in ["x", "dup", "y", "dup"] {
        kv.write(
            db,
            KeyCommand::ListPush {
                key: key("l"),
                value: item.into(),
                left: false,
            },
        )
        .await
        .unwrap();
    }
    kv.write(
        db,
        KeyCommand::ListPush {
            key: key("l"),
            value: "first".into(),
            left: true,
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::ListRemove {
            key: key("l"),
            index: 4,
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::ListSet {
            key: key("l"),
            index: 1,
            value: "changed".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        kv.read(db, &key("l"), ReadRange::default())
            .await
            .unwrap()
            .data,
        KeyData::List {
            items: vec![text("first"), text("changed"), text("dup"), text("y")],
            start: 0,
            total: 4
        }
    );
    assert!(
        kv.write(
            db,
            KeyCommand::ListRemove {
                key: key("l"),
                index: 99
            }
        )
        .await
        .is_err()
    );

    kv.write(
        db,
        KeyCommand::SetAdd {
            key: key("set"),
            member: "a".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::SetAdd {
            key: key("set"),
            member: "b".into(),
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::SetRemove {
            key: key("set"),
            member: "a".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(kv.read(db, &key("set"), ReadRange::default()).await.unwrap().data, KeyData::Set { members, total: 1, .. } if members == vec![text("b")])
    );

    kv.write(
        db,
        KeyCommand::ZsetAdd {
            key: key("z"),
            member: "m".into(),
            score: 1.5,
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::ZsetAdd {
            key: key("z"),
            member: "m".into(),
            score: 3.0,
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::ZsetAdd {
            key: key("z"),
            member: "n".into(),
            score: 2.0,
        },
    )
    .await
    .unwrap();
    kv.write(
        db,
        KeyCommand::ZsetRemove {
            key: key("z"),
            member: "n".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        kv.read(db, &key("z"), ReadRange::default())
            .await
            .unwrap()
            .data,
        KeyData::Zset {
            members: vec![(text("m"), 3.0)],
            start: 0,
            total: 1
        }
    );
    assert!(
        kv.write(
            db,
            KeyCommand::ZsetAdd {
                key: key("z"),
                member: "bad".into(),
                score: f64::NAN
            }
        )
        .await
        .is_err()
    );

    // The wrong kind of command for a key is the server's error, passed on.
    match kv
        .write(
            db,
            KeyCommand::HashSet {
                key: key("s"),
                field: "f".into(),
                value: "v".into(),
            },
        )
        .await
        .unwrap_err()
    {
        Error::Database { code, .. } => assert_eq!(code.as_deref(), Some("WRONGTYPE")),
        other => panic!("unexpected error: {other:?}"),
    }

    // Renaming does not overwrite another key.
    kv.write(
        db,
        KeyCommand::SetString {
            key: key("other"),
            value: "kept".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        kv.write(
            db,
            KeyCommand::Rename {
                key: key("s"),
                to: key("other")
            }
        )
        .await
        .is_err()
    );
    kv.write(
        db,
        KeyCommand::Rename {
            key: key("s"),
            to: key("renamed"),
        },
    )
    .await
    .unwrap();
    assert!(kv.read(db, &key("s"), ReadRange::default()).await.is_err());
    assert_eq!(
        kv.read(db, &key("renamed"), ReadRange::default())
            .await
            .unwrap()
            .data,
        KeyData::String {
            value: text("two words")
        }
    );
    assert_eq!(
        kv.read(db, &key("other"), ReadRange::default())
            .await
            .unwrap()
            .data,
        KeyData::String {
            value: text("kept")
        }
    );

    for name in ["h", "l", "set", "z", "renamed", "other"] {
        kv.write(db, KeyCommand::Delete { key: key(name) })
            .await
            .unwrap();
    }
    assert_eq!(kv.keyspace(db).await.unwrap().keys, 0);
}
