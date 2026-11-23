//! Finding a named instance's port through the SQL Server Browser service
//! (UDP 1434). The protocol is one datagram each way: `0x04` and the instance
//! name ask for one instance, and the answer is `0x05`, a two-byte length and
//! `ServerName;…;InstanceName;…;tcp;<port>;;` text.

use std::time::Duration;

use dbclient_core::{Error, Result};
use tokio::net::UdpSocket;

/// The port the Browser service listens on.
pub const BROWSER_PORT: u16 = 1434;

const WAIT: Duration = Duration::from_secs(3);

/// The TCP port of `instance` on the server at `browser` (host and port).
pub async fn instance_port(browser: &str, instance: &str) -> Result<u16> {
    let local = if browser.starts_with('[') {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = UdpSocket::bind(local)
        .await
        .map_err(|e| Error::Connect(e.to_string()))?;
    socket
        .connect(browser)
        .await
        .map_err(|e| Error::Connect(format!("{browser}: {e}")))?;
    let mut ask = vec![0x04];
    ask.extend_from_slice(instance.as_bytes());
    ask.push(0);
    socket
        .send(&ask)
        .await
        .map_err(|e| Error::Connect(format!("{browser}: {e}")))?;
    let mut answer = vec![0; 4096];
    let size = tokio::time::timeout(WAIT, socket.recv(&mut answer))
        .await
        .map_err(|_| no_browser(browser, instance))?
        .map_err(|_| no_browser(browser, instance))?;
    parse(&answer[..size], instance)
}

fn no_browser(browser: &str, instance: &str) -> Error {
    Error::Connect(format!(
        "the SQL Server Browser service at {browser} did not answer for instance {instance}; \
         is it running, or is UDP 1434 blocked? Give the instance's port instead"
    ))
}

/// The instance's TCP port from a Browser answer.
fn parse(answer: &[u8], instance: &str) -> Result<u16> {
    let bad =
        || Error::Connect("the SQL Server Browser service sent an answer it could not read".into());
    if answer.len() < 3 || answer[0] != 0x05 {
        return Err(bad());
    }
    let text = String::from_utf8_lossy(&answer[3..]);
    // Each instance is a run of key;value pairs, and runs end with ";;".
    for entry in text.split(";;") {
        let parts: Vec<&str> = entry.split(';').collect();
        let value = |key: &str| {
            parts
                .chunks(2)
                .find(|pair| pair.len() == 2 && pair[0].eq_ignore_ascii_case(key))
                .map(|pair| pair[1])
        };
        if !value("InstanceName").is_some_and(|name| name.eq_ignore_ascii_case(instance)) {
            continue;
        }
        return match value("tcp") {
            Some(port) => port.trim().parse().map_err(|_| bad()),
            None => Err(Error::Connect(format!(
                "instance {instance} does not accept TCP connections"
            ))),
        };
    }
    Err(Error::Connect(format!(
        "the server has no instance named {instance}"
    )))
}

/// `server\instance` split in two; a name without a backslash has no instance.
pub fn split_instance(host: &str) -> (&str, Option<&str>) {
    match host.split_once('\\') {
        Some((server, instance)) if !instance.trim().is_empty() => (server, Some(instance.trim())),
        Some((server, _)) => (server, None),
        None => (host, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> Vec<u8> {
        let mut out = vec![0x05];
        out.extend_from_slice(&(text.len() as u16).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn reads_the_port_of_the_instance_asked_for() {
        let text = "ServerName;DB1;InstanceName;MSSQLSERVER;IsClustered;No;Version;16.0.1000.6;tcp;1433;;\
                    ServerName;DB1;InstanceName;SQLEXPRESS;IsClustered;No;Version;16.0.1000.6;tcp;49172;np;\\\\DB1\\pipe\\MSSQL$SQLEXPRESS\\sql\\query;;";
        assert_eq!(parse(&answer(text), "sqlexpress").unwrap(), 49172);
        assert_eq!(parse(&answer(text), "MSSQLSERVER").unwrap(), 1433);
        assert!(
            parse(&answer(text), "OTHER")
                .unwrap_err()
                .to_string()
                .contains("no instance named OTHER")
        );
        let pipes_only = "ServerName;DB1;InstanceName;PIPES;IsClustered;No;Version;16.0;np;\\\\DB1\\pipe\\sql\\query;;";
        assert!(parse(&answer(pipes_only), "PIPES").is_err());
        assert!(parse(b"\x04nonsense", "X").is_err());
    }

    #[test]
    fn splits_server_and_instance() {
        assert_eq!(
            split_instance("db1\\SQLEXPRESS"),
            ("db1", Some("SQLEXPRESS"))
        );
        assert_eq!(split_instance("db1\\"), ("db1", None));
        assert_eq!(split_instance("db1"), ("db1", None));
    }

    #[tokio::test]
    async fn asks_the_browser_service_over_udp() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap().to_string();
        let replier = tokio::spawn(async move {
            let mut buf = [0; 256];
            let (size, from) = server.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..size], b"\x04SQLEXPRESS\x00");
            let text =
                "ServerName;H;InstanceName;SQLEXPRESS;IsClustered;No;Version;16.0;tcp;50123;;";
            server.send_to(&answer(text), from).await.unwrap();
        });
        assert_eq!(instance_port(&address, "SQLEXPRESS").await.unwrap(), 50123);
        replier.await.unwrap();

        // Nobody answers: the error says what to do instead.
        let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = silent.local_addr().unwrap().to_string();
        let error = instance_port(&address, "X").await.unwrap_err().to_string();
        assert!(
            error.contains("Give the instance's port instead"),
            "{error}"
        );
    }
}
