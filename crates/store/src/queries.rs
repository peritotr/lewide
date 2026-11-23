//! Queries the user saved, per connection.

use std::path::PathBuf;
use std::sync::Mutex;

use dbclient_core::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    /// Empty for a query that is being saved for the first time.
    pub id: String,
    /// Id of the saved connection it belongs to.
    pub connection_id: String,
    pub name: String,
    pub sql: String,
}

pub struct QueryStore {
    path: PathBuf,
    writing: Mutex<()>,
}

impl QueryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writing: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Vec<SavedQuery>> {
        match std::fs::read_to_string(&self.path) {
            // An unreadable file is an error, not an empty list: saving over it would lose it.
            Ok(text) => serde_json::from_str(&text).map_err(|e| self.file_error(&e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(self.file_error(&e)),
        }
    }

    fn write(&self, queries: &[SavedQuery]) -> Result<()> {
        let json = serde_json::to_string_pretty(queries).map_err(|e| self.file_error(&e))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.file_error(&e))?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(|e| self.file_error(&e))?;
        std::fs::rename(&temporary, &self.path).map_err(|e| self.file_error(&e))
    }

    /// The connection's queries, by name.
    pub fn list(&self, connection_id: &str) -> Result<Vec<SavedQuery>> {
        let mut queries: Vec<_> = self
            .read()?
            .into_iter()
            .filter(|q| q.connection_id == connection_id)
            .collect();
        queries.sort_by_key(|q| q.name.to_lowercase());
        Ok(queries)
    }

    /// Adds the query (empty id) or replaces the one with the same id.
    pub fn save(&self, mut query: SavedQuery) -> Result<SavedQuery> {
        let _writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        query.name = query.name.trim().to_owned();
        if query.name.is_empty() {
            return Err(Error::InvalidConfig("a saved query needs a name".into()));
        }
        let mut all = self.read()?;
        if query.id.is_empty() {
            query.id = uuid::Uuid::new_v4().to_string();
        }
        match all.iter_mut().find(|q| q.id == query.id) {
            Some(existing) => *existing = query.clone(),
            None => all.push(query.clone()),
        }
        self.write(&all)?;
        Ok(query)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        let mut all = self.read()?;
        all.retain(|q| q.id != id);
        self.write(&all)
    }

    /// Removes every query of a connection that is being removed.
    pub fn delete_for_connection(&self, connection_id: &str) -> Result<()> {
        let _writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        let mut all = self.read()?;
        if all.iter().any(|q| q.connection_id == connection_id) {
            all.retain(|q| q.connection_id != connection_id);
            self.write(&all)?;
        }
        Ok(())
    }

    fn file_error(&self, e: &dyn std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_lists_replaces_and_deletes() {
        let path =
            std::env::temp_dir().join(format!("dbclient-queries-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = QueryStore::new(&path);
        let query = |connection: &str, name: &str| SavedQuery {
            id: String::new(),
            connection_id: connection.into(),
            name: name.into(),
            sql: "SELECT 1".into(),
        };
        assert!(store.list("a").unwrap().is_empty());

        let top = store.save(query("a", "  Top customers ")).unwrap();
        store.save(query("a", "daily revenue")).unwrap();
        store.save(query("b", "other connection")).unwrap();
        assert_eq!(top.name, "Top customers");
        let names: Vec<_> = store
            .list("a")
            .unwrap()
            .into_iter()
            .map(|q| q.name)
            .collect();
        assert_eq!(names, ["daily revenue", "Top customers"]);

        // Same id: replaced in place.
        store
            .save(SavedQuery {
                sql: "SELECT 2".into(),
                ..top.clone()
            })
            .unwrap();
        let listed = store.list("a").unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].sql, "SELECT 2");
        assert!(store.save(query("a", "   ")).is_err());

        store.delete(&top.id).unwrap();
        assert_eq!(store.list("a").unwrap().len(), 1);
        store.delete_for_connection("a").unwrap();
        assert!(store.list("a").unwrap().is_empty());
        assert_eq!(store.list("b").unwrap().len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
