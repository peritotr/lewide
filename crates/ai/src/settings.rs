//! The providers the user set up. Their settings live in a JSON file; their
//! API keys live in the system credential store, like database passwords.

use std::path::PathBuf;
use std::sync::Mutex;

use dbclient_core::{Error, Result};
use dbclient_store::SecretStore;
use serde::{Deserialize, Serialize};

use crate::{ProviderConfig, ProviderKind, ai_error};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Used when a request does not name a provider.
    #[serde(default)]
    pub default_provider: Option<String>,
}

/// A provider as listed for the interface: settings, and whether a key is stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderEntry {
    #[serde(flatten)]
    pub config: ProviderConfig,
    pub has_key: bool,
    pub is_default: bool,
}

pub struct AiStore {
    path: PathBuf,
    secrets: Box<dyn SecretStore>,
    writing: Mutex<()>,
}

/// Keys share the credential store with connection passwords; the prefix keeps them apart.
fn key_name(id: &str) -> String {
    format!("ai:{id}")
}

impl AiStore {
    pub fn new(path: impl Into<PathBuf>, secrets: impl SecretStore + 'static) -> Self {
        Self {
            path: path.into(),
            secrets: Box::new(secrets),
            writing: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<AiSettings> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| self.file_error(&e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AiSettings::default()),
            Err(e) => Err(self.file_error(&e)),
        }
    }

    fn write(&self, settings: &AiSettings) -> Result<()> {
        let json = serde_json::to_string_pretty(settings).map_err(|e| self.file_error(&e))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.file_error(&e))?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(|e| self.file_error(&e))?;
        std::fs::rename(&temporary, &self.path).map_err(|e| self.file_error(&e))
    }

    pub fn list(&self) -> Result<Vec<ProviderEntry>> {
        let settings = self.read()?;
        let default = default_of(&settings);
        settings
            .providers
            .into_iter()
            .map(|config| {
                Ok(ProviderEntry {
                    has_key: self.secrets.get(&key_name(&config.id))?.is_some(),
                    is_default: default.as_deref() == Some(config.id.as_str()),
                    config,
                })
            })
            .collect()
    }

    /// Adds a provider (empty id) or updates one. `api_key`: `None` leaves the
    /// stored key as it is, an empty string removes it, anything else replaces it.
    pub fn save(&self, mut config: ProviderConfig, api_key: Option<&str>) -> Result<ProviderEntry> {
        let _writing = self.lock();
        let mut settings = self.read()?;
        if config.kind == ProviderKind::OpenAiCompatible
            && config
                .base_url
                .as_deref()
                .is_none_or(|u| u.trim().is_empty())
        {
            return Err(Error::InvalidConfig(
                "an OpenAI-compatible provider needs its address".into(),
            ));
        }
        if config.id.is_empty() {
            config.id = uuid::Uuid::new_v4().to_string();
        }
        match api_key {
            None => {}
            Some("") => self.secrets.delete(&key_name(&config.id))?,
            Some(key) => self.secrets.set(&key_name(&config.id), key.trim())?,
        }
        match settings.providers.iter_mut().find(|p| p.id == config.id) {
            Some(existing) => *existing = config.clone(),
            None => settings.providers.push(config.clone()),
        }
        self.write(&settings)?;
        Ok(ProviderEntry {
            has_key: self.secrets.get(&key_name(&config.id))?.is_some(),
            is_default: default_of(&settings).as_deref() == Some(config.id.as_str()),
            config,
        })
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _writing = self.lock();
        let mut settings = self.read()?;
        settings.providers.retain(|p| p.id != id);
        if settings.default_provider.as_deref() == Some(id) {
            settings.default_provider = None;
        }
        self.write(&settings)?;
        self.secrets.delete(&key_name(id))
    }

    pub fn set_default(&self, id: &str) -> Result<()> {
        let _writing = self.lock();
        let mut settings = self.read()?;
        if !settings.providers.iter().any(|p| p.id == id) {
            return Err(ai_error("not_configured", "that provider no longer exists"));
        }
        settings.default_provider = Some(id.to_owned());
        self.write(&settings)
    }

    /// The provider to use and its key: the one named, else the default.
    pub fn resolve(&self, id: Option<&str>) -> Result<(ProviderConfig, Option<String>)> {
        let settings = self.read()?;
        let wanted = id.map(str::to_owned).or_else(|| default_of(&settings));
        let config = wanted
            .and_then(|id| settings.providers.into_iter().find(|p| p.id == id))
            .ok_or_else(|| ai_error("not_configured", "no AI provider is set up yet"))?;
        let key = self.secrets.get(&key_name(&config.id))?;
        Ok((config, key))
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

/// The chosen default, or the only provider when there is exactly one.
fn default_of(settings: &AiSettings) -> Option<String> {
    settings
        .default_provider
        .clone()
        .filter(|id| settings.providers.iter().any(|p| &p.id == id))
        .or_else(|| settings.providers.first().map(|p| p.id.clone()))
}
