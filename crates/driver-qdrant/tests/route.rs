//! How requests are addressed, checked against a stand-in server on this
//! machine. Needs no Qdrant: the stand-in records what it was sent.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use dbclient_core::{ConnectionConfig, Driver, Error};
use dbclient_driver_qdrant::QdrantDriver;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers the two requests a connection attempt makes and keeps their heads.
async fn stand_in() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buffer = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buffer[..n]),
                }
            }
            let head = String::from_utf8_lossy(&head).to_lowercase();
            let body = if head.starts_with("get / ") {
                r#"{"title":"qdrant - vector search engine","version":"9.9.9"}"#
            } else {
                r#"{"result":{"collections":[]},"status":"ok","time":0.0}"#
            };
            log.lock().unwrap().push(head);
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    (addr, seen)
}

fn has_line(head: &str, line: &str) -> bool {
    head.lines().any(|l| l == line)
}

#[tokio::test]
async fn a_route_replaces_the_address_and_keeps_the_name() {
    // A name that does not resolve, and an address nothing listens on: the
    // connection can only succeed by going to the route.
    for (host, port, expected_host) in [
        ("qdrant.invalid", Some(7000), "host: qdrant.invalid:7000"),
        ("192.0.2.10", None, "host: 192.0.2.10:6333"),
        // The port a browser would leave out is left out here too.
        ("qdrant.invalid", Some(80), "host: qdrant.invalid"),
    ] {
        let (route, seen) = stand_in().await;
        let config = ConnectionConfig {
            driver: "qdrant".into(),
            host: Some(host.into()),
            port,
            password: Some("route-test-key".into()),
            route: Some(route),
            ..ConnectionConfig::default()
        };
        let conn = QdrantDriver.connect(&config).await.unwrap();
        assert_eq!(
            conn.server_version().await.unwrap().as_deref(),
            Some("9.9.9")
        );

        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("get / http/1.1"), "{}", seen[0]);
        assert!(
            seen[1].starts_with("get /collections http/1.1"),
            "{}",
            seen[1]
        );
        for head in seen.iter() {
            assert!(has_line(head, expected_host), "{host}: {head}");
            assert!(has_line(head, "api-key: route-test-key"), "{head}");
        }
    }
}

#[tokio::test]
async fn without_a_route_host_and_port_are_used() {
    let (addr, seen) = stand_in().await;
    let config = ConnectionConfig {
        driver: "qdrant".into(),
        host: Some("127.0.0.1".into()),
        port: Some(addr.port()),
        ..ConnectionConfig::default()
    };
    QdrantDriver.connect(&config).await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(has_line(&seen[0], &format!("host: {addr}")), "{}", seen[0]);
    // No key configured, no header sent.
    assert!(!seen[0].contains("api-key"), "{}", seen[0]);
}

#[tokio::test]
async fn nothing_listening_is_a_connection_error() {
    // Bound and dropped: the port is free and almost certainly stays so.
    let addr = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };
    let config = ConnectionConfig {
        driver: "qdrant".into(),
        host: Some("127.0.0.1".into()),
        port: Some(addr.port()),
        ..ConnectionConfig::default()
    };
    let Err(error) = QdrantDriver.connect(&config).await else {
        panic!("connected to a closed port");
    };
    assert!(matches!(error, Error::Connect(_)), "{error:?}");
}
