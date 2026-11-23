use std::path::PathBuf;
use std::sync::Arc;

use dbclient_core::{ConnectionConfig, Error, Result, SshAuth, SshConfig};
use dbclient_store::{
    ConnectionFile, ConnectionStore, Environment, Keep, KeyringSecrets, MemorySecrets,
    SavedConnection, SecretFlags, SecretPolicy, SecretStore, Secrets,
};

/// A settings file of its own per test, removed when the test ends.
struct TempFile(PathBuf);

impl TempFile {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("dbclient-store-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempFile(dir.join("nested").join("connections.json"))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap().parent().unwrap());
    }
}

/// Lets a test look into the secrets the store wrote.
struct Shared(Arc<MemorySecrets>);

impl SecretStore for Shared {
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.0.set(id, secret)
    }
    fn get(&self, id: &str) -> Result<Option<String>> {
        self.0.get(id)
    }
    fn delete(&self, id: &str) -> Result<()> {
        self.0.delete(id)
    }
}

fn store(file: &TempFile) -> (ConnectionStore, Arc<MemorySecrets>) {
    let secrets = Arc::new(MemorySecrets::default());
    (
        ConnectionStore::new(&file.0, Shared(secrets.clone())),
        secrets,
    )
}

fn full_config() -> ConnectionConfig {
    ConnectionConfig {
        driver: "postgres".into(),
        host: Some("db.internal".into()),
        port: Some(5432),
        user: Some("app".into()),
        password: Some("db-secret".into()),
        database: Some("shop".into()),
        ssh: Some(SshConfig {
            host: "bastion.internal".into(),
            port: Some(22),
            user: "ops".into(),
            auth: SshAuth::KeyFile {
                path: "/keys/id_ed25519".into(),
                passphrase: Some("key-secret".into()),
            },
            known_hosts_file: None,
            accept_fingerprint: None,
            remember_host: false,
        }),
        ..ConnectionConfig::default()
    }
}

fn connection(name: &str, config: ConnectionConfig) -> SavedConnection {
    SavedConnection {
        id: String::new(),
        engine: None,
        name: name.into(),
        environment: Environment::Production,
        colour: 1,
        config,
        requires: SecretFlags::default(),
        secrets: SecretPolicy::None,
    }
}

#[test]
fn secrets_go_to_the_secret_store_and_never_to_the_file() {
    let file = TempFile::new("split");
    let (store, secrets) = store(&file);

    let saved = store
        .save(connection("acme-prod", full_config()), Keep::ALL, false)
        .unwrap();
    assert!(!saved.id.is_empty());
    assert_eq!(saved.secrets, SecretPolicy::Keychain);
    // What comes back, and what is listed, carries no secrets.
    assert_eq!(Secrets::of(&saved.config), Secrets::default());
    assert_eq!(store.list().unwrap(), vec![saved.clone()]);

    assert_eq!(
        saved.requires,
        SecretFlags {
            password: true,
            ssh_password: false,
            ssh_passphrase: true
        }
    );
    let on_disk = std::fs::read_to_string(&file.0).unwrap();
    assert!(
        !on_disk.contains("db-secret") && !on_disk.contains("key-secret"),
        "{on_disk}"
    );
    assert!(on_disk.contains("bastion.internal"));
    assert!(
        secrets
            .get(&saved.id)
            .unwrap()
            .unwrap()
            .contains("db-secret")
    );

    // Opening it puts everything back together.
    assert_eq!(store.resolve(&saved.id, None).unwrap(), full_config());
}

#[test]
fn a_connection_without_stored_secrets_asks_for_them() {
    let file = TempFile::new("ask");
    let (store, secrets) = store(&file);

    let saved = store
        .save(connection("staging", full_config()), Keep::NONE, false)
        .unwrap();
    assert_eq!(saved.secrets, SecretPolicy::Ask);
    assert_eq!(secrets.get(&saved.id).unwrap(), None);
    assert!(
        matches!(store.resolve(&saved.id, None), Err(Error::CredentialsRequired(id)) if id == saved.id)
    );

    let entered = Secrets {
        password: Some("typed".into()),
        ssh_passphrase: Some("typed-key".into()),
        ..Secrets::default()
    };
    let config = store.resolve(&saved.id, Some(&entered)).unwrap();
    assert_eq!(config.password.as_deref(), Some("typed"));
    assert_eq!(Secrets::of(&config), entered);
}

#[test]
fn a_connection_without_passwords_needs_none() {
    let file = TempFile::new("none");
    let (store, _) = store(&file);
    let config = ConnectionConfig {
        driver: "sqlite".into(),
        database: Some("/data/app.db".into()),
        ..ConnectionConfig::default()
    };
    let saved = store
        .save(connection("local file", config.clone()), Keep::ALL, false)
        .unwrap();
    assert_eq!(saved.secrets, SecretPolicy::None);
    assert_eq!(store.resolve(&saved.id, None).unwrap(), config);
}

#[test]
fn a_missing_keychain_entry_asks_instead_of_failing() {
    let file = TempFile::new("missing");
    let (store, secrets) = store(&file);
    let saved = store
        .save(connection("acme-prod", full_config()), Keep::ALL, false)
        .unwrap();
    secrets.delete(&saved.id).unwrap();
    assert!(matches!(
        store.resolve(&saved.id, None),
        Err(Error::CredentialsRequired(_))
    ));
}

#[test]
fn saving_again_replaces_and_deleting_removes_the_secret_too() {
    let file = TempFile::new("replace");
    let (store, secrets) = store(&file);
    let first = store
        .save(connection("one", full_config()), Keep::ALL, false)
        .unwrap();
    let second = store
        .save(connection("two", full_config()), Keep::ALL, false)
        .unwrap();
    assert_ne!(first.id, second.id);

    // Same id: replaced in place, and switching to "ask" removes the stored secret.
    let mut renamed = connection("one renamed", full_config());
    renamed.id = first.id.clone();
    store.save(renamed, Keep::NONE, false).unwrap();
    let names: Vec<_> = store.list().unwrap().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["one renamed", "two"]);
    assert_eq!(secrets.get(&first.id).unwrap(), None);

    store.delete(&second.id).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(secrets.get(&second.id).unwrap(), None);
    assert!(matches!(
        store.resolve(&second.id, None),
        Err(Error::Storage(_))
    ));
}

#[test]
fn only_the_chosen_secrets_are_stored() {
    let file = TempFile::new("partial");
    let (store, secrets) = store(&file);
    // Keep the database password, ask for the key passphrase.
    let saved = store
        .save(
            connection("partial", full_config()),
            Keep {
                password: true,
                ssh: false,
            },
            false,
        )
        .unwrap();
    assert_eq!(saved.secrets, SecretPolicy::Ask);
    let stored = secrets.get(&saved.id).unwrap().unwrap();
    assert!(
        stored.contains("db-secret") && !stored.contains("key-secret"),
        "{stored}"
    );
    assert!(matches!(
        store.resolve(&saved.id, None),
        Err(Error::CredentialsRequired(_))
    ));

    // Typing the missing one is enough; the stored one is filled in.
    let typed = Secrets {
        ssh_passphrase: Some("key-secret".into()),
        ..Secrets::default()
    };
    assert_eq!(
        store.resolve(&saved.id, Some(&typed)).unwrap(),
        full_config()
    );

    // Asking to remember it stops the prompt.
    let remembered = store.remember(&saved.id, &typed).unwrap();
    assert_eq!(remembered.secrets, SecretPolicy::Keychain);
    assert_eq!(store.resolve(&saved.id, None).unwrap(), full_config());
}

#[test]
fn editing_with_empty_secrets_leaves_them_unchanged() {
    let file = TempFile::new("edit");
    let (store, _) = store(&file);
    let saved = store
        .save(connection("acme-prod", full_config()), Keep::ALL, false)
        .unwrap();

    // The edit form sends the settings back without passwords, with a new host.
    let mut edited = saved.clone();
    edited.name = "acme-prod-2".into();
    edited.config.host = Some("db2.internal".into());
    let after = store.save(edited, Keep::ALL, true).unwrap();
    assert_eq!(after.secrets, SecretPolicy::Keychain);
    let config = store.resolve(&after.id, None).unwrap();
    assert_eq!(config.host.as_deref(), Some("db2.internal"));
    assert_eq!(config.password.as_deref(), Some("db-secret"));
    assert_eq!(Secrets::of(&config), Secrets::of(&full_config()));

    // Typing a new password replaces only that one.
    let mut edited = after.clone();
    edited.config.password = Some("rotated".into());
    let after = store.save(edited, Keep::ALL, true).unwrap();
    let config = store.resolve(&after.id, None).unwrap();
    assert_eq!(config.password.as_deref(), Some("rotated"));
    assert_eq!(
        Secrets::of(&config).ssh_passphrase.as_deref(),
        Some("key-secret")
    );

    // Removing the tunnel drops the need for its passphrase.
    let mut edited = after.clone();
    edited.config.ssh = None;
    let after = store.save(edited, Keep::ALL, true).unwrap();
    assert_eq!(
        after.requires,
        SecretFlags {
            password: true,
            ..SecretFlags::default()
        }
    );
}

#[test]
fn approved_fingerprint_is_remembered() {
    let file = TempFile::new("fingerprint");
    let (store, _) = store(&file);
    let saved = store
        .save(connection("acme-prod", full_config()), Keep::ALL, false)
        .unwrap();
    store.set_fingerprint(&saved.id, "SHA256:abc").unwrap();
    let config = store.resolve(&saved.id, None).unwrap();
    assert_eq!(
        config.ssh.unwrap().accept_fingerprint.as_deref(),
        Some("SHA256:abc")
    );
}

#[test]
fn a_damaged_file_is_reported_and_left_alone() {
    let file = TempFile::new("damaged");
    let (store, _) = store(&file);
    std::fs::create_dir_all(file.0.parent().unwrap()).unwrap();
    std::fs::write(&file.0, "{ not json").unwrap();

    assert!(matches!(store.list(), Err(Error::Storage(_))));
    assert!(matches!(
        store.save(connection("x", full_config()), Keep::ALL, false),
        Err(Error::Storage(_))
    ));
    assert_eq!(std::fs::read_to_string(&file.0).unwrap(), "{ not json");
}

#[test]
fn a_missing_file_is_an_empty_list() {
    let file = TempFile::new("empty");
    let (store, _) = store(&file);
    assert_eq!(store.list().unwrap(), vec![]);
}

/// Touches the real credential store of this machine, so it runs only on request:
/// `cargo test -p dbclient-store -- --ignored`.
#[test]
#[ignore]
fn system_credential_store_round_trip() {
    let secrets = KeyringSecrets::new("dbclient-store-test");
    let id = format!("round-trip-{}", std::process::id());
    assert_eq!(secrets.get(&id).unwrap(), None);
    secrets.set(&id, "s3cret ✓").unwrap();
    assert_eq!(secrets.get(&id).unwrap().as_deref(), Some("s3cret ✓"));
    secrets.delete(&id).unwrap();
    assert_eq!(secrets.get(&id).unwrap(), None);
    secrets.delete(&id).unwrap();
}

#[test]
fn exported_connections_carry_no_secrets_and_import_as_new_ones() {
    let file = TempFile::new("export");
    let (here, _) = store(&file);
    let mut config = full_config();
    config.params.insert("apiKey".into(), "es-key".into());
    config.params.insert("authSource".into(), "admin".into());
    let saved = here
        .save(connection("acme-prod", config), Keep::ALL, false)
        .unwrap();
    here.save(
        connection("local", ConnectionConfig::default()),
        Keep::NONE,
        false,
    )
    .unwrap();

    let exported = here.export(std::slice::from_ref(&saved.id)).unwrap();
    assert_eq!(exported.connections.len(), 1);
    let json = serde_json::to_string(&exported).unwrap();
    for secret in ["db-secret", "key-secret", "es-key"] {
        assert!(!json.contains(secret), "{json}");
    }
    assert!(json.contains("authSource") && json.contains("bastion.internal"));
    assert_eq!(here.export(&[]).unwrap().connections.len(), 2);

    // Read on another computer: a new id, and the passwords are asked for.
    let other = TempFile::new("import");
    let (elsewhere, _) = store(&other);
    let back: ConnectionFile = serde_json::from_str(&json).unwrap();
    let added = elsewhere.import(back).unwrap();
    assert_eq!(added.len(), 1);
    assert_ne!(added[0].id, saved.id);
    assert_eq!(added[0].name, "acme-prod");
    assert_eq!(added[0].requires, saved.requires);
    assert_eq!(added[0].secrets, SecretPolicy::Ask);
    assert_eq!(elsewhere.list().unwrap(), added);

    let newer = ConnectionFile {
        lewide_connections: 99,
        connections: Vec::new(),
    };
    assert!(matches!(elsewhere.import(newer), Err(Error::Storage(_))));
}
