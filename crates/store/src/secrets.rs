use std::collections::HashMap;
use std::sync::Mutex;

use dbclient_core::{Error, Result};

/// A place to keep one secret string per saved connection.
pub trait SecretStore: Send + Sync {
    fn set(&self, id: &str, secret: &str) -> Result<()>;
    fn get(&self, id: &str) -> Result<Option<String>>;
    /// Deleting an entry that does not exist is not an error.
    fn delete(&self, id: &str) -> Result<()>;
}

/// The operating system's credential store: Keychain on macOS, Credential
/// Manager on Windows, the Secret Service on Linux.
pub struct KeyringSecrets {
    service: String,
}

impl KeyringSecrets {
    /// `service` groups the entries; use the application's identifier.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, id).map_err(keychain_error)
    }
}

impl SecretStore for KeyringSecrets {
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.entry(id)?.set_password(secret).map_err(keychain_error)
    }

    fn get(&self, id: &str) -> Result<Option<String>> {
        match self.entry(id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn delete(&self, id: &str) -> Result<()> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

fn keychain_error(e: keyring::Error) -> Error {
    Error::Storage(format!("system credential store: {e}"))
}

/// Keeps secrets in memory. For tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemorySecrets {
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.map().insert(id.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<String>> {
        Ok(self.map().get(id).cloned())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.map().remove(id);
        Ok(())
    }
}
