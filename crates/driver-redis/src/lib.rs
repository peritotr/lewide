//! Redis driver, also used for wire-compatible stores (Valkey, KeyDB, Dragonfly).
//!
//! Besides the key-value operations the interface uses, a query tab can send
//! commands as text, the way redis-cli does.

mod resp;

use std::time::Instant;

use async_trait::async_trait;
use dbclient_core::kv::{
    KeyCommand, KeyData, KeyDetails, KeyEntry, KeyValue, KeyspaceInfo, ReadRange, ScanPage,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, Connection, ConnectionConfig, DataModel,
    DbObject, Driver, DriverInfo, Error, Namespace, ObjectDescription, QueryRequest, QueryResult,
    Result, SqlDialect, TlsMode, Value,
};
use resp::Reply;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// A plain or a TLS socket; the protocol code reads and writes either the same way.
trait Socket: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Socket for T {}

/// Hash fields and set members read in one step.
const DEFAULT_PAGE: u32 = 200;
/// Values larger than this are not loaded into the editor.
const MAX_STRING_BYTES: usize = 16 * 1024 * 1024;

pub struct RedisDriver;

#[async_trait]
impl Driver for RedisDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "redis",
            name: "Redis",
            model: DataModel::KeyValue,
            capabilities: Capabilities::default(),
            default_port: Some(6379),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let host = config
            .host
            .as_deref()
            .filter(|h| !h.trim().is_empty())
            .unwrap_or("localhost");
        let port = config.port.unwrap_or(6379);
        // The database is a number in Redis; an empty field means the first one.
        let db = match config
            .database
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            None => 0,
            Some(text) => text.trim_start_matches("db").parse::<u32>().map_err(|_| {
                Error::InvalidConfig(format!("the database is a number in Redis, not {text:?}"))
            })?,
        };

        let stream = match config.route {
            // Through an SSH tunnel the socket goes to the tunnel's local end.
            Some(route) => TcpStream::connect(route).await,
            None => TcpStream::connect((host, port)).await,
        }
        .map_err(|e| Error::Connect(format!("{host}:{port}: {e}")))?;
        let _ = stream.set_nodelay(true);

        let socket: Box<dyn Socket> = if config.tls.mode == TlsMode::Off {
            Box::new(stream)
        } else {
            // Through a route the certificate is still checked against the real host.
            let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                .map_err(|e| Error::InvalidConfig(format!("host {host:?}: {e}")))?;
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(
                dbclient_tls::client_config(&config.tls, None)?,
            ));
            Box::new(
                connector
                    .connect(name, stream)
                    .await
                    .map_err(|e| Error::Connect(format!("TLS with {host}:{port}: {e}")))?,
            )
        };

        let connection = RedisConnection {
            stream: Mutex::new(BufStream::new(socket)),
            db,
        };
        if let Some(password) = config.password.as_deref().filter(|p| !p.is_empty()) {
            let mut auth = vec![b"AUTH".to_vec()];
            if let Some(user) = config.user.as_deref().filter(|u| !u.is_empty()) {
                auth.push(user.as_bytes().to_vec());
            }
            auth.push(password.as_bytes().to_vec());
            if let Reply::Error(message) = connection
                .raw(vec![auth])
                .await
                .map_err(connect_err)?
                .remove(0)
            {
                return Err(Error::Connect(message));
            }
        }
        // Without a password a protected server answers the first command with NOAUTH.
        match connection
            .raw(vec![vec![b"PING".to_vec()]])
            .await
            .map_err(connect_err)?
            .remove(0)
        {
            Reply::Error(message) => Err(Error::Connect(message)),
            _ => Ok(Box::new(connection)),
        }
    }
}

fn connect_err(error: Error) -> Error {
    Error::Connect(error.to_string())
}

struct RedisConnection {
    /// One connection, one command sequence at a time.
    stream: Mutex<BufStream<Box<dyn Socket>>>,
    /// The database the connection settings name.
    db: u32,
}

fn arg(text: impl AsRef<[u8]>) -> Vec<u8> {
    text.as_ref().to_vec()
}

macro_rules! cmd {
    ($($part:expr),+ $(,)?) => {
        vec![$(arg($part)),+]
    };
}

impl RedisConnection {
    /// Sends the commands together and reads one reply for each.
    async fn raw(&self, commands: Vec<Vec<Vec<u8>>>) -> Result<Vec<Reply>> {
        let mut stream = self.stream.lock().await;
        let lost = |e: std::io::Error| Error::ConnectionLost(e.to_string());
        for command in &commands {
            stream
                .write_all(&resp::encode(command))
                .await
                .map_err(lost)?;
        }
        stream.flush().await.map_err(lost)?;
        let mut replies = Vec::with_capacity(commands.len());
        for _ in &commands {
            replies.push(resp::read(&mut *stream).await.map_err(lost)?);
        }
        Ok(replies)
    }

    /// Runs commands in one database. The SELECT goes out with them, so
    /// another caller's database choice cannot come in between.
    async fn in_db(&self, db: u32, mut commands: Vec<Vec<Vec<u8>>>) -> Result<Vec<Reply>> {
        commands.insert(0, cmd!["SELECT", db.to_string()]);
        let mut replies = self.raw(commands).await?;
        if let Reply::Error(message) = replies.remove(0) {
            return Err(server_error(message));
        }
        Ok(replies)
    }

    /// One command whose error reply is an error here.
    async fn one(&self, db: u32, command: Vec<Vec<u8>>) -> Result<Reply> {
        match self.in_db(db, vec![command]).await?.remove(0) {
            Reply::Error(message) => Err(server_error(message)),
            reply => Ok(reply),
        }
    }

    async fn info(&self, section: &str) -> Result<String> {
        Ok(text(&self.one(self.db, cmd!["INFO", section]).await?).unwrap_or_default())
    }
}

/// Redis errors start with a word in capitals that names the kind (`ERR`, `WRONGTYPE`, `NOAUTH`).
fn server_error(message: String) -> Error {
    let code = message
        .split_whitespace()
        .next()
        .filter(|w| w.chars().all(|c| c.is_ascii_uppercase()))
        .map(str::to_owned);
    Error::Database {
        code,
        message,
        position: None,
        line: None,
        hint: None,
        detail: None,
    }
}

fn text(reply: &Reply) -> Option<String> {
    match reply {
        Reply::Simple(s) => Some(s.clone()),
        Reply::Bulk(Some(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Reply::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

fn int(reply: &Reply) -> Option<i64> {
    match reply {
        Reply::Int(i) => Some(*i),
        other => text(other)?.parse().ok(),
    }
}

fn items(reply: Reply) -> Vec<Reply> {
    match reply {
        Reply::Array(Some(items)) => items,
        _ => Vec::new(),
    }
}

/// Bytes as a value: text when they are UTF-8, binary otherwise.
fn value(reply: &Reply) -> Value {
    match reply {
        Reply::Bulk(Some(bytes)) => match std::str::from_utf8(bytes) {
            Ok(s) => Value::Text(s.to_owned()),
            Err(_) => Value::Bytes(bytes.clone()),
        },
        Reply::Bulk(None) | Reply::Array(None) => Value::Null,
        Reply::Int(i) => Value::Int(*i),
        Reply::Simple(s) | Reply::Error(s) => Value::Text(s.clone()),
        Reply::Array(Some(inner)) => Value::Json(json(&Reply::Array(Some(inner.clone())))),
    }
}

fn json(reply: &Reply) -> serde_json::Value {
    match reply {
        Reply::Array(Some(inner)) => serde_json::Value::Array(inner.iter().map(json).collect()),
        Reply::Int(i) => serde_json::Value::from(*i),
        Reply::Bulk(None) | Reply::Array(None) => serde_json::Value::Null,
        other => serde_json::Value::String(text(other).unwrap_or_default()),
    }
}

/// `PTTL`: -1 is no expiry, -2 is no such key.
fn ttl(reply: &Reply) -> Option<i64> {
    int(reply).filter(|ms| *ms >= 0)
}

/// `db0:keys=3,expires=0,avg_ttl=0` lines of `INFO keyspace`.
fn keyspace_lines(info: &str) -> Vec<(u32, u64)> {
    info.lines()
        .filter_map(|line| {
            let (name, rest) = line.trim().split_once(':')?;
            let db = name.strip_prefix("db")?.parse().ok()?;
            let keys = rest
                .split(',')
                .find_map(|part| part.strip_prefix("keys="))?
                .parse()
                .ok()?;
            Some((db, keys))
        })
        .collect()
}

fn info_field(info: &str, name: &str) -> Option<String> {
    info.lines().find_map(|line| {
        line.trim()
            .strip_prefix(name)?
            .strip_prefix(':')
            .map(str::to_owned)
    })
}

/// A command as text, to show what was run. Arguments with spaces are quoted.
fn command_text(command: &[Vec<u8>]) -> String {
    command
        .iter()
        .map(|part| {
            let s = String::from_utf8_lossy(part);
            if s.is_empty() || s.contains(char::is_whitespace) || s.contains('"') {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                s.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[async_trait]
impl KeyValue for RedisConnection {
    async fn keyspace(&self, db: u32) -> Result<KeyspaceInfo> {
        let keys = int(&self.one(db, cmd!["DBSIZE"]).await?)
            .unwrap_or(0)
            .max(0) as u64;
        let memory_bytes =
            info_field(&self.info("memory").await?, "used_memory").and_then(|m| m.parse().ok());
        let databases = keyspace_lines(&self.info("keyspace").await?);
        Ok(KeyspaceInfo {
            keys,
            memory_bytes,
            databases,
        })
    }

    async fn scan(
        &self,
        db: u32,
        pattern: &str,
        kind: Option<&str>,
        cursor: &str,
        count: u32,
    ) -> Result<ScanPage> {
        let pattern = if pattern.trim().is_empty() {
            "*"
        } else {
            pattern
        };
        let mut scan = cmd![
            "SCAN",
            cursor,
            "MATCH",
            pattern,
            "COUNT",
            count.clamp(1, 10_000).to_string()
        ];
        if let Some(kind) = kind.filter(|k| !k.is_empty()) {
            scan.push(arg("TYPE"));
            scan.push(arg(kind));
        }
        let mut reply = items(self.one(db, scan).await?);
        if reply.len() != 2 {
            return Err(Error::Query("unexpected SCAN reply".into()));
        }
        let names: Vec<Vec<u8>> = items(reply.remove(1))
            .into_iter()
            .filter_map(|r| match r {
                Reply::Bulk(Some(bytes)) => Some(bytes),
                _ => None,
            })
            .collect();
        let cursor = text(&reply[0]).unwrap_or_else(|| "0".into());

        // The type and time to live of each key, asked together.
        let mut commands = Vec::with_capacity(names.len() * 2);
        for name in &names {
            commands.push(vec![arg("TYPE"), name.clone()]);
            commands.push(vec![arg("PTTL"), name.clone()]);
        }
        let details = if commands.is_empty() {
            Vec::new()
        } else {
            self.in_db(db, commands).await?
        };
        let keys = names
            .iter()
            .zip(details.chunks(2))
            .filter_map(|(name, pair)| {
                let kind = text(&pair[0])?;
                // Gone between the scan and the question.
                (kind != "none").then(|| KeyEntry {
                    key: String::from_utf8_lossy(name).into_owned(),
                    kind,
                    ttl_ms: ttl(&pair[1]),
                })
            })
            .collect();
        Ok(ScanPage { keys, cursor })
    }

    async fn read(&self, db: u32, key: &str, range: ReadRange) -> Result<KeyDetails> {
        let head = self
            .in_db(
                db,
                vec![
                    cmd!["TYPE", key],
                    cmd!["PTTL", key],
                    cmd!["MEMORY", "USAGE", key],
                    cmd!["OBJECT", "ENCODING", key],
                ],
            )
            .await?;
        let kind = text(&head[0]).unwrap_or_default();
        if kind == "none" {
            return Err(Error::Query(format!("no such key: {key}")));
        }
        let limit = if range.limit == 0 {
            DEFAULT_PAGE
        } else {
            range.limit.min(5000)
        } as u64;
        let count = |reply: &Reply| int(reply).unwrap_or(0).max(0) as u64;

        let data = match kind.as_str() {
            "string" => {
                let length = count(&self.one(db, cmd!["STRLEN", key]).await?);
                if length as usize > MAX_STRING_BYTES {
                    return Err(Error::Query(format!(
                        "the value is {length} bytes; values over {MAX_STRING_BYTES} bytes are not loaded"
                    )));
                }
                KeyData::String {
                    value: value(&self.one(db, cmd!["GET", key]).await?),
                }
            }
            "hash" => {
                let replies = self
                    .in_db(
                        db,
                        vec![
                            cmd!["HLEN", key],
                            cmd![
                                "HSCAN",
                                key,
                                range.cursor.as_deref().unwrap_or("0"),
                                "COUNT",
                                limit.to_string()
                            ],
                        ],
                    )
                    .await?;
                let mut scan = items(replies[1].clone());
                let pairs = if scan.len() == 2 {
                    items(scan.remove(1))
                } else {
                    Vec::new()
                };
                KeyData::Hash {
                    fields: pairs
                        .chunks(2)
                        .filter(|p| p.len() == 2)
                        .map(|p| (text(&p[0]).unwrap_or_default(), value(&p[1])))
                        .collect(),
                    total: count(&replies[0]),
                }
            }
            "list" => {
                let stop = range.offset + limit - 1;
                let replies = self
                    .in_db(
                        db,
                        vec![
                            cmd!["LLEN", key],
                            cmd!["LRANGE", key, range.offset.to_string(), stop.to_string()],
                        ],
                    )
                    .await?;
                KeyData::List {
                    items: items(replies[1].clone()).iter().map(value).collect(),
                    start: range.offset,
                    total: count(&replies[0]),
                }
            }
            "set" => {
                let replies = self
                    .in_db(
                        db,
                        vec![
                            cmd!["SCARD", key],
                            cmd![
                                "SSCAN",
                                key,
                                range.cursor.as_deref().unwrap_or("0"),
                                "COUNT",
                                limit.to_string()
                            ],
                        ],
                    )
                    .await?;
                let mut scan = items(replies[1].clone());
                let (cursor, members) = if scan.len() == 2 {
                    let members = items(scan.remove(1));
                    (text(&scan[0]).unwrap_or_else(|| "0".into()), members)
                } else {
                    ("0".into(), Vec::new())
                };
                KeyData::Set {
                    members: members.iter().map(value).collect(),
                    cursor,
                    total: count(&replies[0]),
                }
            }
            "zset" => {
                let stop = range.offset + limit - 1;
                let order = if range.descending {
                    "ZREVRANGE"
                } else {
                    "ZRANGE"
                };
                let replies = self
                    .in_db(
                        db,
                        vec![
                            cmd!["ZCARD", key],
                            cmd![
                                order,
                                key,
                                range.offset.to_string(),
                                stop.to_string(),
                                "WITHSCORES"
                            ],
                        ],
                    )
                    .await?;
                KeyData::Zset {
                    members: items(replies[1].clone())
                        .chunks(2)
                        .filter(|p| p.len() == 2)
                        .map(|p| {
                            (
                                value(&p[0]),
                                text(&p[1]).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN),
                            )
                        })
                        .collect(),
                    start: range.offset,
                    total: count(&replies[0]),
                }
            }
            _ => KeyData::Other,
        };
        Ok(KeyDetails {
            key: key.to_owned(),
            kind,
            ttl_ms: ttl(&head[1]),
            // Both are optional commands: an older or restricted server answers with an error.
            size_bytes: int(&head[2]).filter(|n| *n >= 0).map(|n| n as u64),
            encoding: match &head[3] {
                Reply::Error(_) => None,
                other => text(other),
            },
            data,
        })
    }

    async fn write(&self, db: u32, command: KeyCommand) -> Result<String> {
        let commands: Vec<Vec<Vec<u8>>> = match command {
            KeyCommand::SetString { key, value } => vec![cmd!["SET", key, value, "KEEPTTL"]],
            KeyCommand::HashSet { key, field, value } => vec![cmd!["HSET", key, field, value]],
            KeyCommand::HashDelete { key, field } => vec![cmd!["HDEL", key, field]],
            KeyCommand::ListPush { key, value, left } => {
                vec![cmd![if left { "LPUSH" } else { "RPUSH" }, key, value]]
            }
            KeyCommand::ListSet { key, index, value } => {
                vec![cmd!["LSET", key, index.to_string(), value]]
            }
            KeyCommand::ListRemove { key, index } => {
                // Redis removes list items by value, not by position: the item is
                // first overwritten with a marker no real value has, then the marker is removed.
                let marker = format!(
                    "\u{0}lewide-removed-{}\u{0}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                );
                vec![
                    cmd!["MULTI"],
                    cmd!["LSET", &key, index.to_string(), &marker],
                    cmd!["LREM", &key, "1", &marker],
                    cmd!["EXEC"],
                ]
            }
            KeyCommand::SetAdd { key, member } => vec![cmd!["SADD", key, member]],
            KeyCommand::SetRemove { key, member } => vec![cmd!["SREM", key, member]],
            KeyCommand::ZsetAdd { key, member, score } => {
                if !score.is_finite() {
                    return Err(Error::Query("a score is a number".into()));
                }
                vec![cmd!["ZADD", key, score.to_string(), member]]
            }
            KeyCommand::ZsetRemove { key, member } => vec![cmd!["ZREM", key, member]],
            KeyCommand::Delete { key } => vec![cmd!["DEL", key]],
            // RENAMENX: an existing key of the new name is not overwritten.
            KeyCommand::Rename { key, to } => vec![cmd!["RENAMENX", key, to]],
            KeyCommand::Expire { key, seconds } => vec![cmd!["EXPIRE", key, seconds.to_string()]],
            KeyCommand::Persist { key } => vec![cmd!["PERSIST", key]],
        };
        let shown = if commands.len() == 4 {
            // The list removal: what it amounts to, not the marker dance.
            format!("{} + {}", command_text(&commands[1][..3]), "LREM")
        } else {
            command_text(&commands[0])
        };
        let renaming = commands[0][0] == b"RENAMENX";
        let replies = self.in_db(db, commands).await?;
        for reply in &replies {
            match reply {
                Reply::Error(message) => return Err(server_error(message.clone())),
                // EXEC answers with the replies of the queued commands.
                Reply::Array(Some(inner)) => {
                    if let Some(Reply::Error(message)) =
                        inner.iter().find(|r| matches!(r, Reply::Error(_)))
                    {
                        return Err(server_error(message.clone()));
                    }
                }
                _ => {}
            }
        }
        if renaming && replies.first() == Some(&Reply::Int(0)) {
            return Err(Error::Query("a key with that name already exists".into()));
        }
        Ok(shown)
    }
}

#[async_trait]
impl Connection for RedisConnection {
    async fn ping(&self) -> Result<()> {
        self.one(self.db, cmd!["PING"]).await.map(|_| ())
    }

    /// Each line is one command, written as for redis-cli.
    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let mut results = Vec::new();
        for line in request
            .statement
            .lines()
            .map(|l| l.trim().trim_end_matches(';'))
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("--"))
        {
            let command = resp::split_command(line).map_err(Error::Query)?;
            if command.is_empty() {
                continue;
            }
            let started = Instant::now();
            let reply = self.one(self.db, command).await?;
            let mut rows: Vec<Vec<Value>> = match &reply {
                Reply::Array(Some(inner)) => inner.iter().map(|item| vec![value(item)]).collect(),
                other => vec![vec![value(other)]],
            };
            let truncated = request.max_rows.is_some_and(|max| rows.len() > max);
            if let Some(max) = request.max_rows {
                rows.truncate(max);
            }
            results.push(QueryResult {
                columns: vec![Column {
                    name: "reply".into(),
                    type_name: None,
                }],
                rows,
                rows_affected: None,
                truncated,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        if results.is_empty() {
            return Err(Error::Query("no command to run".into()));
        }
        Ok(results)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let info = self.info("server").await?;
        // Valkey and others keep redis_version for compatibility and add their own.
        Ok(info_field(&info, "valkey_version")
            .map(|v| format!("Valkey {v}"))
            .or_else(|| info_field(&info, "redis_version")))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some(format!("db{}", self.db)))
    }

    /// The databases that hold keys, and the one the connection names.
    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let mut numbers: Vec<u32> = keyspace_lines(&self.info("keyspace").await?)
            .into_iter()
            .map(|(db, _)| db)
            .collect();
        if !numbers.contains(&self.db) {
            numbers.push(self.db);
        }
        numbers.sort_unstable();
        Ok(numbers
            .into_iter()
            .map(|db| Namespace {
                name: format!("db{db}"),
            })
            .collect())
    }

    /// Keys are listed by scanning (see [`KeyValue::scan`]), not as objects.
    async fn list_objects(&self, _namespace: Option<&str>) -> Result<Vec<DbObject>> {
        Ok(Vec::new())
    }

    async fn describe_object(
        &self,
        _namespace: Option<&str>,
        _name: &str,
    ) -> Result<ObjectDescription> {
        Err(Error::Unsupported("keys have no columns"))
    }

    /// Unused: nothing here is written as SQL.
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    async fn apply_changes(&self, _changes: &ChangeSet) -> Result<ApplyResult> {
        Err(Error::Unsupported("row changes"))
    }

    fn quote_ident(&self, name: &str) -> String {
        name.to_owned()
    }

    fn key_value(&self) -> Option<&dyn KeyValue> {
        Some(self)
    }

    async fn close(&self) -> Result<()> {
        let _ = self.raw(vec![cmd!["QUIT"]]).await;
        Ok(())
    }
}
