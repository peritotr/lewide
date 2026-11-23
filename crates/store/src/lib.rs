//! Saved connections.
//!
//! Settings live in one JSON file; passwords live in the operating system's
//! credential store (Keychain, Credential Manager, Secret Service) and never
//! in that file. [`ConnectionConfig`] cannot serialize its secret fields, so
//! the split does not depend on this crate remembering to strip them.

mod queries;
mod secrets;

use std::path::PathBuf;
use std::sync::Mutex;

use dbclient_core::{ConnectionConfig, Error, Result, SshAuth};
use serde::{Deserialize, Serialize};

pub use queries::{QueryStore, SavedQuery};
pub use secrets::{KeyringSecrets, MemorySecrets, SecretStore};

const FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    #[default]
    Local,
    Staging,
    Production,
}

/// Where a saved connection's passwords come from when it is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPolicy {
    /// The connection has no passwords.
    None,
    /// All of them are in the credential store.
    Keychain,
    /// At least one is not stored and has to be asked for.
    Ask,
}

/// One flag per kind of secret a connection can have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFlags {
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub ssh_password: bool,
    #[serde(default)]
    pub ssh_passphrase: bool,
}

/// Which secrets the user wants kept in the credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keep {
    pub password: bool,
    /// The SSH password or key passphrase, whichever the connection uses.
    pub ssh: bool,
}

impl Keep {
    pub const ALL: Keep = Keep {
        password: true,
        ssh: true,
    };
    pub const NONE: Keep = Keep {
        password: false,
        ssh: false,
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    /// The database type the user picked, as the interface's two-letter code
    /// (`Pg`, `Cr`, `Ma`). Several types share a driver; this tells them apart.
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub environment: Environment,
    /// Index into the connection marker palette, 1 to 4.
    pub colour: u8,
    pub config: ConnectionConfig,
    /// The secrets this connection needs in order to open.
    #[serde(default)]
    pub requires: SecretFlags,
    pub secrets: SecretPolicy,
}

/// The secret parts of a [`ConnectionConfig`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_passphrase: Option<String>,
}

impl Secrets {
    pub fn of(config: &ConnectionConfig) -> Secrets {
        let filled = |s: &String| (!s.is_empty()).then(|| s.clone());
        let (ssh_password, ssh_passphrase) = match config.ssh.as_ref().map(|ssh| &ssh.auth) {
            Some(SshAuth::Password { password }) => (filled(password), None),
            Some(SshAuth::KeyFile { passphrase, .. }) => {
                (None, passphrase.as_ref().and_then(filled))
            }
            None => (None, None),
        };
        Secrets {
            password: config.password.as_ref().and_then(filled),
            ssh_password,
            ssh_passphrase,
        }
    }

    fn flags(&self) -> SecretFlags {
        SecretFlags {
            password: self.password.is_some(),
            ssh_password: self.ssh_password.is_some(),
            ssh_passphrase: self.ssh_passphrase.is_some(),
        }
    }

    /// `self` where it has a value, `fallback` elsewhere.
    fn or(self, fallback: Secrets) -> Secrets {
        Secrets {
            password: self.password.or(fallback.password),
            ssh_password: self.ssh_password.or(fallback.ssh_password),
            ssh_passphrase: self.ssh_passphrase.or(fallback.ssh_passphrase),
        }
    }

    fn only(self, keep: Keep) -> Secrets {
        Secrets {
            password: self.password.filter(|_| keep.password),
            ssh_password: self.ssh_password.filter(|_| keep.ssh),
            ssh_passphrase: self.ssh_passphrase.filter(|_| keep.ssh),
        }
    }

    /// Whether every secret named in `required` is present.
    fn covers(&self, required: SecretFlags) -> bool {
        (!required.password || self.password.is_some())
            && (!required.ssh_password || self.ssh_password.is_some())
            && (!required.ssh_passphrase || self.ssh_passphrase.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.ssh_password.is_none() && self.ssh_passphrase.is_none()
    }

    /// Puts the secrets back into a config that was stored without them.
    pub fn apply_to(&self, config: &mut ConnectionConfig) {
        if self.password.is_some() {
            config.password = self.password.clone();
        }
        match config.ssh.as_mut().map(|ssh| &mut ssh.auth) {
            Some(SshAuth::Password { password }) => {
                if let Some(value) = &self.ssh_password {
                    *password = value.clone();
                }
            }
            Some(SshAuth::KeyFile { passphrase, .. }) if self.ssh_passphrase.is_some() => {
                *passphrase = self.ssh_passphrase.clone();
            }
            _ => {}
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileContents {
    version: u32,
    connections: Vec<SavedConnection>,
}

/// Connections written to a file to be read on another computer, or kept as
/// a copy. Passwords are never in it: they stay in this computer's credential
/// store, and an imported connection asks for them when it is first opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionFile {
    /// Names what the file is and its format version.
    pub lewide_connections: u32,
    pub connections: Vec<SavedConnection>,
}

const EXPORT_VERSION: u32 = 1;

/// Driver options that carry a credential (Elasticsearch's API key); they are
/// left out of an exported file like the passwords.
const SECRET_PARAMS: &[&str] = &["apiKey"];

pub struct ConnectionStore {
    path: PathBuf,
    secrets: Box<dyn SecretStore>,
    /// Serializes read-modify-write cycles on the file.
    writing: Mutex<()>,
}

impl ConnectionStore {
    /// `path` is the settings file; its directory is created on the first save.
    pub fn new(path: impl Into<PathBuf>, secrets: impl SecretStore + 'static) -> Self {
        Self {
            path: path.into(),
            secrets: Box::new(secrets),
            writing: Mutex::new(()),
        }
    }

    pub fn list(&self) -> Result<Vec<SavedConnection>> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(self.file_error(&e)),
        };
        // An unreadable file is an error, not an empty list: saving over it would lose it.
        let contents: FileContents =
            serde_json::from_str(&text).map_err(|e| self.file_error(&e))?;
        if contents.version > FILE_VERSION {
            return Err(Error::Storage(format!(
                "{} was written by a newer version of the application",
                self.path.display()
            )));
        }
        Ok(contents.connections)
    }

    /// Adds the connection, or replaces the one with the same id. An empty id
    /// gets a new one. `config` carries the passwords as the user entered them;
    /// the kinds named in `keep` go to the credential store and the rest are
    /// asked for when the connection is opened.
    ///
    /// With `unchanged_if_empty`, a secret left empty keeps what the existing
    /// entry has: that is how an edit form leaves a password untouched without
    /// ever loading it.
    pub fn save(
        &self,
        mut connection: SavedConnection,
        keep: Keep,
        unchanged_if_empty: bool,
    ) -> Result<SavedConnection> {
        let _writing = self.lock();
        let mut all = self.list()?;
        if connection.id.is_empty() {
            connection.id = uuid::Uuid::new_v4().to_string();
        }
        let existing = all.iter().find(|c| c.id == connection.id);

        let entered = Secrets::of(&connection.config);
        let (secrets, mut requires) = match existing {
            Some(existing) if unchanged_if_empty => (
                entered.clone().or(self.stored(&existing.id)?),
                existing.requires,
            ),
            _ => (entered.clone(), SecretFlags::default()),
        };
        let entered = entered.flags();
        requires.password |= entered.password;
        requires.ssh_password |= entered.ssh_password;
        requires.ssh_passphrase |= entered.ssh_passphrase;
        // A secret for an SSH method the connection no longer uses is not needed any more.
        match connection.config.ssh.as_ref().map(|ssh| &ssh.auth) {
            Some(SshAuth::Password { .. }) => requires.ssh_passphrase = false,
            Some(SshAuth::KeyFile { .. }) => requires.ssh_password = false,
            None => {
                requires.ssh_password = false;
                requires.ssh_passphrase = false;
            }
        }

        let to_store = secrets.only(keep);
        connection.requires = requires;
        connection.secrets = policy(requires, &to_store);
        self.put_secrets(&connection.id, &to_store)?;

        match all.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection.clone(),
            None => all.push(connection.clone()),
        }
        self.write(all)?;
        // Hand back what was stored: the same connection without its secrets.
        Secrets::default().clear(&mut connection.config);
        Ok(connection)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _writing = self.lock();
        let mut all = self.list()?;
        all.retain(|c| c.id != id);
        self.write(all)?;
        self.secrets.delete(id)
    }

    /// The full settings for opening a saved connection. `entered` are secrets
    /// the user typed; they take precedence over stored ones.
    pub fn resolve(&self, id: &str, entered: Option<&Secrets>) -> Result<ConnectionConfig> {
        let saved = self.find(id)?;
        let secrets = entered.cloned().unwrap_or_default().or(self.stored(id)?);
        // A stored secret can be gone: removed by the user, or the settings
        // file was copied from another machine.
        if !secrets.covers(saved.requires) {
            return Err(Error::CredentialsRequired(id.to_owned()));
        }
        let mut config = saved.config;
        secrets.apply_to(&mut config);
        Ok(config)
    }

    /// Adds secrets the user typed to the credential store, so the connection
    /// stops asking for them.
    pub fn remember(&self, id: &str, entered: &Secrets) -> Result<SavedConnection> {
        let _writing = self.lock();
        let mut all = self.list()?;
        let secrets = entered.clone().or(self.stored(id)?);
        self.put_secrets(id, &secrets)?;
        let saved = all
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Storage(format!("no saved connection with id {id}")))?;
        saved.secrets = policy(saved.requires, &secrets);
        let updated = saved.clone();
        self.write(all)?;
        Ok(updated)
    }

    fn find(&self, id: &str) -> Result<SavedConnection> {
        self.list()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Storage(format!("no saved connection with id {id}")))
    }

    /// The secrets in the credential store for this connection; empty if there are none.
    pub fn stored(&self, id: &str) -> Result<Secrets> {
        match self.secrets.get(id)? {
            Some(json) => serde_json::from_str(&json).map_err(|e| Error::Storage(e.to_string())),
            None => Ok(Secrets::default()),
        }
    }

    fn put_secrets(&self, id: &str, secrets: &Secrets) -> Result<()> {
        if secrets.is_empty() {
            return self.secrets.delete(id);
        }
        let json = serde_json::to_string(secrets).map_err(|e| Error::Storage(e.to_string()))?;
        self.secrets.set(id, &json)
    }

    /// The connections with these ids (all of them when `ids` is empty),
    /// ready to be written to a file: no passwords, no API keys.
    pub fn export(&self, ids: &[String]) -> Result<ConnectionFile> {
        let connections = self
            .list()?
            .into_iter()
            .filter(|c| ids.is_empty() || ids.contains(&c.id))
            .map(|mut c| {
                for key in SECRET_PARAMS {
                    c.config.params.remove(*key);
                }
                c.secrets = policy(c.requires, &Secrets::default());
                c
            })
            .collect();
        Ok(ConnectionFile {
            lewide_connections: EXPORT_VERSION,
            connections,
        })
    }

    /// Adds the file's connections as new ones, beside those already saved.
    /// Nothing is stored in the credential store: a connection that needs a
    /// password asks for it when it is opened.
    pub fn import(&self, file: ConnectionFile) -> Result<Vec<SavedConnection>> {
        if file.lewide_connections > EXPORT_VERSION {
            return Err(Error::Storage(
                "the file was written by a newer version of the application".into(),
            ));
        }
        let _writing = self.lock();
        let mut all = self.list()?;
        let added: Vec<SavedConnection> = file
            .connections
            .into_iter()
            .map(|mut c| {
                c.id = uuid::Uuid::new_v4().to_string();
                // A file edited by hand may carry a password; it is not kept.
                Secrets::default().clear(&mut c.config);
                c.secrets = policy(c.requires, &Secrets::default());
                c.colour = c.colour.clamp(1, 4);
                c
            })
            .collect();
        all.extend(added.iter().cloned());
        self.write(all)?;
        Ok(added)
    }

    /// Records the SSH host key fingerprint the user approved for this connection.
    pub fn set_fingerprint(&self, id: &str, fingerprint: &str) -> Result<()> {
        let _writing = self.lock();
        let mut all = self.list()?;
        if let Some(ssh) = all
            .iter_mut()
            .find(|c| c.id == id)
            .and_then(|c| c.config.ssh.as_mut())
        {
            ssh.accept_fingerprint = Some(fingerprint.to_owned());
            self.write(all)?;
        }
        Ok(())
    }

    fn write(&self, connections: Vec<SavedConnection>) -> Result<()> {
        let contents = FileContents {
            version: FILE_VERSION,
            connections,
        };
        let json = serde_json::to_string_pretty(&contents).map_err(|e| self.file_error(&e))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.file_error(&e))?;
        }
        // Write beside the file and rename, so a crash never leaves it half written.
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(|e| self.file_error(&e))?;
        std::fs::rename(&temporary, &self.path).map_err(|e| self.file_error(&e))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn file_error(&self, e: &dyn std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.path.display()))
    }
}

fn policy(requires: SecretFlags, stored: &Secrets) -> SecretPolicy {
    if requires == SecretFlags::default() {
        SecretPolicy::None
    } else if stored.covers(requires) {
        SecretPolicy::Keychain
    } else {
        SecretPolicy::Ask
    }
}

impl Secrets {
    /// Removes every secret from `config`.
    fn clear(&self, config: &mut ConnectionConfig) {
        config.password = None;
        match config.ssh.as_mut().map(|ssh| &mut ssh.auth) {
            Some(SshAuth::Password { password }) => password.clear(),
            Some(SshAuth::KeyFile { passphrase, .. }) => *passphrase = None,
            None => {}
        }
    }
}
