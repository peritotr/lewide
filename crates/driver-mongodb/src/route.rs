//! Sends the driver's connections to a fixed socket address while it keeps
//! using the server's real name.
//!
//! An SSH tunnel gives us a local address (`ConnectionConfig::route`) that
//! leads to the server. The MongoDB driver takes one host name and uses it
//! both to open the socket and to check the TLS certificate; it has no
//! setting to separate the two. It can, however, connect through a SOCKS5
//! proxy, to which it hands the host name unresolved. So a minimal proxy runs
//! here, inside the process: it accepts the driver's connection requests,
//! ignores the address asked for, and connects to the route instead.
//!
//! It listens on a loopback port without authentication. That adds no
//! exposure: the route it forwards to is itself a loopback port any local
//! process can reach.

use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The running proxy. Dropping it stops new connections; the ones already
/// open end when the driver closes them.
pub struct RouteProxy {
    pub port: u16,
    accepting: JoinHandle<()>,
}

impl Drop for RouteProxy {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

pub async fn start(route: SocketAddr) -> io::Result<RouteProxy> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let accepting = tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            tokio::spawn(async move {
                // A failed connection shows up in the driver as its own error.
                let _ = relay(client, route).await;
            });
        }
    });
    Ok(RouteProxy { port, accepting })
}

/// The server side of a SOCKS5 CONNECT (RFC 1928), then plain forwarding.
async fn relay(mut client: TcpStream, route: SocketAddr) -> io::Result<()> {
    let refuse = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());

    // Greeting: version, number of methods, methods. Answer: no authentication.
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(refuse("not SOCKS5"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    client.read_exact(&mut methods).await?;
    client.write_all(&[5, 0]).await?;

    // Request: version, command, reserved, address type, address, port.
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let address_len = match request[3] {
        1 => 4,
        3 => client.read_u8().await? as usize,
        4 => 16,
        _ => return Err(refuse("unknown address type")),
    };
    // The address is read and dropped: every connection goes to the route.
    let mut address = vec![0u8; address_len + 2];
    client.read_exact(&mut address).await?;

    // Reply: version, status, reserved, then a bound address nobody uses.
    let reply = |status: u8| [5, status, 0, 1, 0, 0, 0, 0, 0, 0];
    if request[1] != 1 {
        client.write_all(&reply(7)).await?; // command not supported
        return Err(refuse("only CONNECT is supported"));
    }
    let mut server = match TcpStream::connect(route).await {
        Ok(server) => server,
        Err(e) => {
            client.write_all(&reply(5)).await?; // connection refused
            return Err(e);
        }
    };
    client.write_all(&reply(0)).await?;
    copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}
