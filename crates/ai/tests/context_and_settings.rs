use dbclient_ai::context::schema_context;
use dbclient_ai::{AiStore, ProviderConfig, ProviderKind};
use dbclient_core::{ConnectionConfig, Driver, Error, QueryRequest};
use dbclient_driver_sqlite::SqliteDriver;
use dbclient_store::{MemorySecrets, SecretStore};

#[tokio::test]
async fn the_schema_description_has_structure_and_no_row_data() {
    let config = ConnectionConfig {
        driver: "sqlite".into(),
        database: Some(":memory:".into()),
        ..ConnectionConfig::default()
    };
    let conn = SqliteDriver.connect(&config).await.unwrap();
    conn.execute(QueryRequest {
        statement: "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE, country TEXT);
                    CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INT NOT NULL, total REAL);
                    CREATE INDEX orders_customer ON orders (customer_id);
                    CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100;
                    INSERT INTO customers VALUES (1, 'secret.person@example.com', 'TR');
                    INSERT INTO orders VALUES (1, 1, 4242.42);"
            .into(),
        max_rows: None,
    })
    .await
    .unwrap();

    let context = schema_context(conn.as_ref(), None, Some("SELECT * FROM orders"))
        .await
        .unwrap();
    assert_eq!(context.namespace.as_deref(), Some("main"));
    assert_eq!((context.tables_total, context.tables_detailed), (3, 3));
    let text = &context.text;

    // The table named in the statement is described first.
    assert!(text.starts_with("TABLE main.orders\n"), "{text}");
    assert!(text.contains("  id INTEGER PRIMARY KEY"));
    assert!(text.contains("  customer_id INT NOT NULL"));
    assert!(text.contains("  INDEX orders_customer (customer_id)"));
    assert!(text.contains("TABLE main.customers"));
    assert!(text.contains("  email TEXT NOT NULL"));
    assert!(text.contains("UNIQUE INDEX"));
    assert!(text.contains("VIEW main.big_orders"));

    // Nothing that is stored in the rows appears.
    assert!(
        !text.contains("secret.person") && !text.contains("4242") && !text.contains("'TR'"),
        "{text}"
    );
}

fn settings_file(test: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("dbclient-ai-{test}-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn anthropic(name: &str) -> ProviderConfig {
    ProviderConfig {
        id: String::new(),
        kind: ProviderKind::Anthropic,
        name: name.into(),
        base_url: None,
        model: None,
    }
}

#[test]
fn keys_stay_out_of_the_settings_file() {
    let file = settings_file("keys");
    let store = AiStore::new(&file, MemorySecrets::default());
    assert!(store.list().unwrap().is_empty());
    assert!(
        matches!(store.resolve(None), Err(Error::Ai { reason, .. }) if reason == "not_configured")
    );

    let saved = store
        .save(anthropic("Anthropic"), Some(" sk-ant-secret-key "))
        .unwrap();
    assert!(saved.has_key && saved.is_default);
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("sk-ant-secret-key")
    );

    // The only provider is the default, and its key comes back trimmed for requests.
    let (config, key) = store.resolve(None).unwrap();
    assert_eq!(config.id, saved.config.id);
    assert_eq!(key.as_deref(), Some("sk-ant-secret-key"));

    // Saving settings without a key leaves the key alone; an empty key removes it.
    let mut renamed = saved.config.clone();
    renamed.model = Some("claude-sonnet-5-5".into());
    assert!(store.save(renamed.clone(), None).unwrap().has_key);
    assert!(!store.save(renamed, Some("")).unwrap().has_key);
    assert_eq!(store.resolve(None).unwrap().1, None);
    let _ = std::fs::remove_file(&file);
}

#[test]
fn default_provider_and_removal() {
    let file = settings_file("default");
    let secrets = std::sync::Arc::new(MemorySecrets::default());
    struct Shared(std::sync::Arc<MemorySecrets>);
    impl SecretStore for Shared {
        fn set(&self, id: &str, secret: &str) -> dbclient_core::Result<()> {
            self.0.set(id, secret)
        }
        fn get(&self, id: &str) -> dbclient_core::Result<Option<String>> {
            self.0.get(id)
        }
        fn delete(&self, id: &str) -> dbclient_core::Result<()> {
            self.0.delete(id)
        }
    }
    let store = AiStore::new(&file, Shared(secrets.clone()));

    let first = store
        .save(anthropic("One"), Some("key-1"))
        .unwrap()
        .config
        .id;
    let second = store
        .save(anthropic("Two"), Some("key-2"))
        .unwrap()
        .config
        .id;
    assert_eq!(store.resolve(None).unwrap().0.id, first);
    store.set_default(&second).unwrap();
    assert_eq!(store.resolve(None).unwrap().0.id, second);
    assert_eq!(
        store.resolve(Some(&first)).unwrap().1.as_deref(),
        Some("key-1")
    );

    // Removing the default falls back to the remaining provider and deletes its key.
    store.delete(&second).unwrap();
    assert_eq!(store.resolve(None).unwrap().0.id, first);
    assert_eq!(secrets.get(&format!("ai:{second}")).unwrap(), None);
    assert!(store.set_default(&second).is_err());

    let compatible = ProviderConfig {
        id: String::new(),
        kind: ProviderKind::OpenAiCompatible,
        name: "Local".into(),
        base_url: None,
        model: None,
    };
    assert!(matches!(
        store.save(compatible, None),
        Err(Error::InvalidConfig(_))
    ));
    let _ = std::fs::remove_file(&file);
}
