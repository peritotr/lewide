//! Tests against the servers started by `scripts/test-servers.sh`. The database
//! servers are addressed by their names inside the Docker network (`pg`,
//! `mysql`), which are reachable only through the SSH server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dbclient_core::{
    Connection, ConnectionConfig, DriverRegistry, Error, QueryRequest, SshAuth, SshConfig,
    TlsConfig, TlsMode, Value,
};

struct Env {
    ssh_host: String,
    ssh_port: u16,
    host_key: String,
    key: String,
    key_encrypted: String,
    key_passphrase: String,
    ca_file: String,
}

fn env() -> Option<Env> {
    let var = |name: &str| std::env::var(name).ok();
    let (ssh_host, ssh_port) = var("DBCLIENT_TEST_SSH_ADDR")?
        .rsplit_once(':')
        .map(|(h, p)| {
            (
                h.to_owned(),
                p.parse().expect("invalid port in DBCLIENT_TEST_SSH_ADDR"),
            )
        })?;
    Some(Env {
        ssh_host,
        ssh_port,
        host_key: var("DBCLIENT_TEST_SSH_HOST_KEY")?,
        key: var("DBCLIENT_TEST_SSH_KEY")?,
        key_encrypted: var("DBCLIENT_TEST_SSH_KEY_ENCRYPTED")?,
        key_passphrase: var("DBCLIENT_TEST_SSH_KEY_PASSPHRASE")?,
        ca_file: var("DBCLIENT_TEST_TLS_CA")?,
    })
}

macro_rules! env_or_skip {
    () => {
        match env() {
            Some(env) => env,
            None => {
                eprintln!("skipped: the DBCLIENT_TEST_SSH_* variables are not set");
                return;
            }
        }
    };
}

fn registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry.register(dbclient_driver_postgres::PostgresDriver);
    registry.register(dbclient_driver_mysql::MysqlDriver);
    registry
}

/// A `known_hosts` file of its own per test, holding `content`.
fn known_hosts(test: &str, content: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "dbclient-known-hosts-{test}-{}",
        std::process::id()
    ));
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

impl Env {
    /// The entry a user would have after connecting to the test server once.
    fn trusted_hosts(&self, test: &str) -> String {
        known_hosts(
            test,
            &format!("[{}]:{} {}\n", self.ssh_host, self.ssh_port, self.host_key),
        )
    }

    fn ssh(&self, auth: SshAuth, known_hosts_file: String) -> SshConfig {
        SshConfig {
            host: self.ssh_host.clone(),
            port: Some(self.ssh_port),
            user: "root".into(),
            auth,
            known_hosts_file: Some(known_hosts_file),
            accept_fingerprint: None,
            remember_host: false,
        }
    }

    fn password(&self) -> SshAuth {
        SshAuth::Password {
            password: "test".into(),
        }
    }
}

fn postgres(ssh: SshConfig) -> ConnectionConfig {
    ConnectionConfig {
        driver: "postgres".into(),
        host: Some("pg".into()),
        user: Some("postgres".into()),
        password: Some("test".into()),
        ssh: Some(ssh),
        ..ConnectionConfig::default()
    }
}

fn mysql(ssh: SshConfig) -> ConnectionConfig {
    ConnectionConfig {
        driver: "mysql".into(),
        host: Some("mysql".into()),
        user: Some("root".into()),
        password: Some("test".into()),
        database: Some("app".into()),
        ssh: Some(ssh),
        ..ConnectionConfig::default()
    }
}

async fn select_one(conn: &dyn Connection) -> Value {
    let request = QueryRequest {
        statement: "SELECT 1".into(),
        max_rows: None,
    };
    conn.execute(request)
        .await
        .unwrap()
        .rows
        .remove(0)
        .remove(0)
}

#[tokio::test]
async fn unknown_host_needs_approval() {
    let env = env_or_skip!();
    let mut ssh = env.ssh(env.password(), known_hosts("unknown", ""));

    let err = dbclient_tunnel::connect(&registry(), &postgres(ssh.clone()))
        .await
        .err()
        .unwrap();
    let Error::UnknownHostKey {
        fingerprint,
        key_type,
    } = err
    else {
        panic!("unexpected error: {err:?}")
    };
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    assert!(
        key_type.starts_with("ssh-")
            || key_type.starts_with("ecdsa-")
            || key_type.starts_with("rsa-"),
        "{key_type}"
    );

    // A different approved fingerprint does not help.
    ssh.accept_fingerprint = Some("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into());
    let err = dbclient_tunnel::connect(&registry(), &postgres(ssh.clone()))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::UnknownHostKey { .. }), "{err:?}");

    ssh.accept_fingerprint = Some(fingerprint);
    let conn = dbclient_tunnel::connect(&registry(), &postgres(ssh))
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));
}

#[tokio::test]
async fn known_host_connects_without_approval() {
    let env = env_or_skip!();
    let ssh = env.ssh(env.password(), env.trusted_hosts("known"));
    let conn = dbclient_tunnel::connect(&registry(), &mysql(ssh))
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));
}

#[tokio::test]
async fn changed_host_key_is_refused_even_if_approved() {
    let env = env_or_skip!();
    // Record the client's public key as if it were the server's: same algorithm, different key.
    let other_key = std::fs::read_to_string(format!("{}.pub", env.key)).unwrap();
    let other_key = other_key
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    let hosts = known_hosts(
        "changed",
        &format!("[{}]:{} {other_key}\n", env.ssh_host, env.ssh_port),
    );

    let mut ssh = env.ssh(env.password(), hosts.clone());
    let err = dbclient_tunnel::connect(&registry(), &postgres(ssh.clone()))
        .await
        .err()
        .unwrap();
    match &err {
        Error::HostKeyChanged {
            line,
            saved,
            received,
            ..
        } => {
            assert_eq!(*line, 1);
            assert!(saved.starts_with("SHA256:") && received.starts_with("SHA256:"));
            assert_ne!(saved, received);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // Learn the real fingerprint from a clean file, then check that approving it is not enough.
    let clean = env.ssh(env.password(), known_hosts("changed-clean", ""));
    let Error::UnknownHostKey { fingerprint, .. } =
        dbclient_tunnel::connect(&registry(), &postgres(clean))
            .await
            .err()
            .unwrap()
    else {
        panic!("expected an unknown host key")
    };
    ssh.accept_fingerprint = Some(fingerprint);
    let err = dbclient_tunnel::connect(&registry(), &postgres(ssh))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::HostKeyChanged { .. }), "{err:?}");
}

#[tokio::test]
async fn an_approved_host_can_be_added_to_known_hosts() {
    let env = env_or_skip!();
    let hosts = known_hosts("remember", "");
    let mut ssh = env.ssh(env.password(), hosts.clone());
    let Error::UnknownHostKey { fingerprint, .. } =
        dbclient_tunnel::connect(&registry(), &postgres(ssh.clone()))
            .await
            .err()
            .unwrap()
    else {
        panic!("expected an unknown host key")
    };

    ssh.accept_fingerprint = Some(fingerprint);
    ssh.remember_host = true;
    dbclient_tunnel::connect(&registry(), &postgres(ssh.clone()))
        .await
        .unwrap();
    assert!(std::fs::read_to_string(&hosts).unwrap().contains("ssh-"));

    // From now on the host is known without any approval.
    ssh.accept_fingerprint = None;
    ssh.remember_host = false;
    let conn = dbclient_tunnel::connect(&registry(), &postgres(ssh))
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));
}

#[tokio::test]
async fn key_file_authentication() {
    let env = env_or_skip!();
    let hosts = env.trusted_hosts("key");

    let plain = SshAuth::KeyFile {
        path: env.key.clone(),
        passphrase: None,
    };
    let conn = dbclient_tunnel::connect(&registry(), &postgres(env.ssh(plain, hosts.clone())))
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));

    let encrypted = |passphrase: Option<&str>| SshAuth::KeyFile {
        path: env.key_encrypted.clone(),
        passphrase: passphrase.map(str::to_owned),
    };
    let conn = dbclient_tunnel::connect(
        &registry(),
        &postgres(env.ssh(encrypted(Some(&env.key_passphrase)), hosts.clone())),
    )
    .await
    .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));

    for wrong in [Some("wrong"), None] {
        let err = dbclient_tunnel::connect(
            &registry(),
            &postgres(env.ssh(encrypted(wrong), hosts.clone())),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(err, Error::InvalidConfig(_)), "{err:?}");
    }
}

#[tokio::test]
async fn wrong_ssh_password_is_a_connect_error() {
    let env = env_or_skip!();
    let auth = SshAuth::Password {
        password: "definitely-wrong".into(),
    };
    let err = dbclient_tunnel::connect(
        &registry(),
        &postgres(env.ssh(auth, env.trusted_hosts("wrongpw"))),
    )
    .await
    .err()
    .unwrap();
    match &err {
        Error::Connect(message) => assert!(message.contains("rejected"), "{message}"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn unreachable_target_reports_the_tunnel_error() {
    let env = env_or_skip!();
    let mut config = postgres(env.ssh(env.password(), env.trusted_hosts("unreachable")));
    config.host = Some("no-such-host".into());
    let err = dbclient_tunnel::connect(&registry(), &config)
        .await
        .err()
        .unwrap();
    match &err {
        Error::Connect(message) => assert!(
            message.contains("SSH tunnel to no-such-host:5432"),
            "{message}"
        ),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn tls_is_verified_against_the_real_server_name() {
    let env = env_or_skip!();
    let tls = TlsConfig {
        mode: TlsMode::Verify,
        ca_file: Some(env.ca_file.clone()),
    };

    // The certificate names `pg` and `mysql`, not the loopback address the driver connects to.
    let mut pg = postgres(env.ssh(env.password(), env.trusted_hosts("tls-pg")));
    pg.tls = tls.clone();
    let conn = dbclient_tunnel::connect(&registry(), &pg).await.unwrap();
    let request = QueryRequest {
        statement: "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()".into(),
        max_rows: None,
    };
    assert_eq!(
        conn.execute(request).await.unwrap().rows[0][0],
        Value::Bool(true)
    );

    let mut my = mysql(env.ssh(env.password(), env.trusted_hosts("tls-mysql")));
    my.tls = tls;
    let conn = dbclient_tunnel::connect(&registry(), &my).await.unwrap();
    let request = QueryRequest {
        statement: "SHOW SESSION STATUS LIKE 'Ssl_cipher'".into(),
        max_rows: None,
    };
    let cipher = conn
        .execute(request)
        .await
        .unwrap()
        .rows
        .remove(0)
        .remove(1);
    assert!(matches!(cipher, Value::Text(c) if !c.is_empty()));
}

#[tokio::test]
async fn cancel_works_through_the_tunnel() {
    let env = env_or_skip!();
    for (config, sleep) in [
        (
            postgres(env.ssh(env.password(), env.trusted_hosts("cancel-pg"))),
            "SELECT pg_sleep(30)",
        ),
        (
            mysql(env.ssh(env.password(), env.trusted_hosts("cancel-mysql"))),
            "SELECT SLEEP(30)",
        ),
    ] {
        let conn: Arc<dyn Connection> = Arc::from(
            dbclient_tunnel::connect(&registry(), &config)
                .await
                .unwrap(),
        );
        let started = Instant::now();
        let runner = {
            let conn = conn.clone();
            let request = QueryRequest {
                statement: sleep.into(),
                max_rows: None,
            };
            tokio::spawn(async move { conn.execute(request).await })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        conn.cancel().await.unwrap();
        let _ = runner.await.unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{}",
            config.driver
        );
        assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));
    }
}

#[tokio::test]
async fn without_ssh_the_connection_is_direct() {
    let Ok(url) = std::env::var("DBCLIENT_TEST_POSTGRES_URL") else {
        eprintln!("skipped: DBCLIENT_TEST_POSTGRES_URL is not set");
        return;
    };
    let parsed: tokio_postgres::Config = url.parse().unwrap();
    let config = ConnectionConfig {
        driver: "postgres".into(),
        host: Some("127.0.0.1".into()),
        port: parsed.get_ports().first().copied(),
        user: Some("postgres".into()),
        password: Some("test".into()),
        ..ConnectionConfig::default()
    };
    let conn = dbclient_tunnel::connect(&registry(), &config)
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));
}

#[tokio::test]
async fn a_saved_connection_opens_with_its_stored_secrets() {
    use dbclient_store::{
        ConnectionStore, Environment, Keep, MemorySecrets, SavedConnection, SecretFlags,
        SecretPolicy,
    };

    let env = env_or_skip!();
    let file = std::env::temp_dir().join(format!("dbclient-saved-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let store = ConnectionStore::new(&file, MemorySecrets::default());

    // Database password and SSH password both come back from the secret store.
    let config = postgres(env.ssh(env.password(), env.trusted_hosts("saved")));
    let saved = store
        .save(
            SavedConnection {
                id: String::new(),
                engine: None,
                name: "pg through ssh".into(),
                environment: Environment::Staging,
                colour: 2,
                config,
                requires: SecretFlags::default(),
                secrets: SecretPolicy::None,
            },
            Keep::ALL,
            false,
        )
        .unwrap();
    assert_eq!(saved.config.password, None);
    assert!(!std::fs::read_to_string(&file).unwrap().contains("test\""));

    let resolved = store.resolve(&saved.id, None).unwrap();
    let conn = dbclient_tunnel::connect(&registry(), &resolved)
        .await
        .unwrap();
    assert_eq!(select_one(conn.as_ref()).await, Value::Int(1));

    // Without the secrets the same settings are refused, which shows they were needed.
    let without = dbclient_tunnel::connect(&registry(), &saved.config).await;
    assert!(without.is_err());
    let _ = std::fs::remove_file(&file);
}
