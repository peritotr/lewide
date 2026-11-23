//! SSH tunnelling for database connections.
//!
//! A tunnel is a listener on a loopback port; every connection made to it is
//! carried to the database server through the SSH server. Drivers need no SSH
//! support of their own: they receive the loopback address as the
//! connection's `route`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use dbclient_core::{
    Connection, ConnectionConfig, DbObject, DriverRegistry, Error, ForeignKey, IndexDescription,
    Namespace, ObjectDescription, QueryRequest, QueryResult, Reference, Result, SshAuth, SshConfig,
    TriggerDescription,
};
use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Connects through `config.ssh` when it is set, directly otherwise.
pub async fn connect(
    registry: &DriverRegistry,
    config: &ConnectionConfig,
) -> Result<Box<dyn Connection>> {
    let Some(ssh) = &config.ssh else {
        return registry.connect(config).await;
    };
    let info = registry
        .info(&config.driver)
        .ok_or_else(|| Error::UnknownDriver(config.driver.clone()))?;
    let port = config
        .port
        .or(info.default_port)
        .filter(|_| !info.capabilities.file_based);
    let Some(port) = port else {
        return Err(Error::InvalidConfig(format!(
            "{} connections cannot go through an SSH tunnel",
            info.name
        )));
    };
    let host = config
        .host
        .as_deref()
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost");

    let tunnel = Tunnel::open(ssh, host, port).await?;
    let routed = ConnectionConfig {
        ssh: None,
        route: Some(tunnel.local_addr()),
        ..config.clone()
    };
    match registry.connect(&routed).await {
        Ok(inner) => Ok(Box::new(Tunneled {
            inner,
            _tunnel: tunnel,
        })),
        // The driver only sees a closed socket; the tunnel knows the actual reason.
        Err(driver_error) => Err(match tunnel.take_error() {
            Some(reason) => Error::Connect(format!("SSH tunnel to {host}:{port}: {reason}")),
            None => driver_error,
        }),
    }
}

/// A loopback listener forwarding to one remote address. Closed on drop.
pub struct Tunnel {
    local_addr: SocketAddr,
    last_error: Arc<Mutex<Option<String>>>,
    accept_loop: JoinHandle<()>,
}

impl Tunnel {
    pub async fn open(ssh: &SshConfig, target_host: &str, target_port: u16) -> Result<Tunnel> {
        let session = Arc::new(open_session(ssh).await?);
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| Error::Internal(format!("cannot open a local port: {e}")))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| Error::Internal(e.to_string()))?;
        let last_error = Arc::new(Mutex::new(None));

        let target_host = target_host.to_owned();
        let errors = last_error.clone();
        let accept_loop = tokio::spawn(async move {
            while let Ok((mut socket, peer)) = listener.accept().await {
                let session = session.clone();
                let target_host = target_host.clone();
                let errors = errors.clone();
                tokio::spawn(async move {
                    let channel = session
                        .channel_open_direct_tcpip(
                            target_host,
                            target_port.into(),
                            "127.0.0.1",
                            peer.port().into(),
                        )
                        .await;
                    match channel {
                        Ok(channel) => {
                            let mut remote = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut socket, &mut remote).await;
                        }
                        Err(e) => *lock(&errors) = Some(e.to_string()),
                    }
                });
            }
        });

        Ok(Tunnel {
            local_addr,
            last_error,
            accept_loop,
        })
    }

    /// The loopback address that leads to the remote server.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Why the most recent forwarded connection could not be opened, if it failed.
    pub fn take_error(&self) -> Option<String> {
        lock(&self.last_error).take()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Ends the listener; the SSH session closes once its last channel is gone.
        self.accept_loop.abort();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn open_session(ssh: &SshConfig) -> Result<client::Handle<HostKeyCheck>> {
    let port = ssh.port.unwrap_or(22);
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..client::Config::default()
    });
    let check = HostKeyCheck {
        host: ssh.host.clone(),
        port,
        known_hosts_file: ssh
            .known_hosts_file
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        accept_fingerprint: ssh.accept_fingerprint.clone(),
        remember_host: ssh.remember_host,
    };

    let connecting = client::connect(config, (ssh.host.as_str(), port), check);
    let mut session = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok(session)) => session,
        Ok(Err(SessionError::Rejected(error))) => return Err(error),
        Ok(Err(SessionError::Ssh(e))) => {
            return Err(Error::Connect(format!(
                "SSH server {}:{port}: {e}",
                ssh.host
            )));
        }
        Err(_) => {
            return Err(Error::Connect(format!(
                "SSH server {}:{port}: timed out",
                ssh.host
            )));
        }
    };

    let ssh_err = |e: russh::Error| Error::Connect(format!("SSH authentication: {e}"));
    let outcome = match &ssh.auth {
        SshAuth::Password { password } => session
            .authenticate_password(ssh.user.as_str(), password.as_str())
            .await
            .map_err(ssh_err)?,
        SshAuth::KeyFile { path, passphrase } => {
            let key = russh::keys::load_secret_key(path, passphrase.as_deref())
                .map_err(|e| Error::InvalidConfig(format!("SSH key file {path}: {e}")))?;
            // RSA keys sign with the strongest hash the server accepts.
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(ssh_err)?
                .flatten();
            session
                .authenticate_publickey(
                    ssh.user.as_str(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(ssh_err)?
        }
    };
    if !outcome.success() {
        return Err(Error::Connect(format!(
            "SSH server {}:{port} rejected the credentials for user {}",
            ssh.host, ssh.user
        )));
    }
    Ok(session)
}

/// Decides whether the SSH server is the one the user expects.
struct HostKeyCheck {
    host: String,
    port: u16,
    known_hosts_file: Option<PathBuf>,
    accept_fingerprint: Option<String>,
    remember_host: bool,
}

impl HostKeyCheck {
    /// The `known_hosts` file in use: the configured one, else `~/.ssh/known_hosts`.
    fn known_hosts_path(&self) -> Option<PathBuf> {
        self.known_hosts_file
            .clone()
            .or_else(|| std::env::home_dir().map(|home| home.join(".ssh").join("known_hosts")))
    }
}

#[derive(Debug)]
enum SessionError {
    Ssh(russh::Error),
    /// The host key check refused the server, with the reason to report.
    Rejected(Error),
}

impl From<russh::Error> for SessionError {
    fn from(e: russh::Error) -> Self {
        SessionError::Ssh(e)
    }
}

impl client::Handler for HostKeyCheck {
    type Error = SessionError;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, SessionError> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            return Err(SessionError::Rejected(Error::Connect(
                "the SSH server presented a host certificate, which is not supported yet".into(),
            )));
        };

        let path = self.known_hosts_path();
        let known = match &path {
            Some(path) if path.exists() => {
                russh::keys::check_known_hosts_path(&self.host, self.port, key, path)
            }
            _ => Ok(false),
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        match known {
            Ok(true) => Ok(true),
            // A different key than the recorded one is never accepted, whatever
            // fingerprint was approved: this is what an interception looks like.
            Err(russh::keys::Error::KeyChanged { line }) => {
                let saved = path
                    .and_then(|p| {
                        russh::keys::known_hosts::known_host_keys_path(&self.host, self.port, p)
                            .ok()
                    })
                    .and_then(|keys| keys.into_iter().find(|(l, _)| *l == line))
                    .map(|(_, recorded)| recorded.fingerprint(HashAlg::Sha256).to_string())
                    .unwrap_or_default();
                Err(SessionError::Rejected(Error::HostKeyChanged {
                    host: self.host.clone(),
                    line,
                    saved,
                    received: fingerprint,
                }))
            }
            // Not recorded (or the file is unreadable): the user has to approve it.
            Ok(false) | Err(_) => {
                if self.accept_fingerprint.as_deref() != Some(fingerprint.as_str()) {
                    return Err(SessionError::Rejected(Error::UnknownHostKey {
                        fingerprint,
                        key_type: key.algorithm().to_string(),
                    }));
                }
                if self.remember_host
                    && let Some(path) = path
                {
                    // Failing to write the file does not undo the user's approval.
                    let _ = russh::keys::known_hosts::learn_known_hosts_path(
                        &self.host, self.port, key, path,
                    );
                }
                Ok(true)
            }
        }
    }
}

/// A connection that keeps its tunnel open for as long as it lives.
struct Tunneled {
    // Declared first so the connection is dropped before the tunnel.
    inner: Box<dyn Connection>,
    _tunnel: Tunnel,
}

#[async_trait]
impl Connection for Tunneled {
    async fn ping(&self) -> Result<()> {
        self.inner.ping().await
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        self.inner.execute_all(request).await
    }

    async fn server_version(&self) -> Result<Option<String>> {
        self.inner.server_version().await
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        self.inner.current_namespace().await
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        self.inner.list_namespaces().await
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        self.inner.list_objects(namespace).await
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        self.inner.describe_object(namespace, name).await
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        self.inner.list_indexes(namespace, table).await
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        self.inner.list_foreign_keys(namespace, table).await
    }

    fn key_value(&self) -> Option<&dyn dbclient_core::kv::KeyValue> {
        self.inner.key_value()
    }

    fn document(&self) -> Option<&dyn dbclient_core::document::DocumentStore> {
        self.inner.document()
    }

    fn vector(&self) -> Option<&dyn dbclient_core::vector::VectorStore> {
        self.inner.vector()
    }

    fn graph(&self) -> Option<&dyn dbclient_core::graph::GraphStore> {
        self.inner.graph()
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        self.inner.list_referencing(namespace, table).await
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        self.inner.list_triggers(namespace, table).await
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        self.inner.object_ddl(namespace, name).await
    }

    fn quote_ident(&self, name: &str) -> String {
        self.inner.quote_ident(name)
    }

    async fn is_alive(&self) -> bool {
        self.inner.is_alive().await
    }

    fn dialect(&self) -> dbclient_core::SqlDialect {
        self.inner.dialect()
    }

    async fn in_transaction(&self) -> Result<bool> {
        self.inner.in_transaction().await
    }

    async fn apply_changes(
        &self,
        changes: &dbclient_core::ChangeSet,
    ) -> Result<dbclient_core::ApplyResult> {
        self.inner.apply_changes(changes).await
    }

    async fn cancel(&self) -> Result<()> {
        self.inner.cancel().await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }
}
