use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid tag name: {0}")]
    InvalidName(String),
    #[error("tag not found: {0}")]
    NotFound(String),
    #[error("tag database is corrupted or uses an unsupported schema version")]
    UnsupportedSchema,
    #[error("cannot open tag database at {0}: {1}")]
    OpenFailed(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagDef {
    pub id: i64,
    pub name: String,
    pub display_token: String,
    pub created_at: i64,
}

pub fn validate_name(name: &str) -> Result<(), TagError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(TagError::InvalidName("empty name".into()));
    }
    if trimmed != name {
        return Err(TagError::InvalidName(
            "leading or trailing whitespace".into(),
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(TagError::InvalidName("control characters".into()));
    }
    if name.chars().count() > 64 {
        return Err(TagError::InvalidName("longer than 64 characters".into()));
    }
    Ok(())
}

fn display_token(name: &str) -> String {
    let mut token: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(6)
        .collect();
    if token.is_empty() {
        token = "tag".to_string();
    }
    token
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

pub struct TagStore {
    conn: Connection,
}

impl TagStore {
    pub fn open_in_memory() -> Result<Self, TagError> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    pub fn open(path: &Path) -> Result<Self, TagError> {
        let conn = Connection::open(path)
            .map_err(|e| TagError::OpenFailed(path.display().to_string(), e.to_string()))?;
        Self::init(conn).map_err(|e| match e {
            TagError::Sqlite(inner) => {
                TagError::OpenFailed(path.display().to_string(), inner.to_string())
            }
            other => other,
        })
    }

    fn init(conn: Connection) -> Result<Self, TagError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 3000i64)?;
        let store = TagStore { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), TagError> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(TagError::UnsupportedSchema);
        }
        if version < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tags (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    display_token TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS path_tags (
                    path_bytes BLOB NOT NULL,
                    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                    device_id INTEGER,
                    inode INTEGER,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (path_bytes, tag_id)
                );",
            )?;
            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    pub fn create_tag(&mut self, name: &str, now: i64) -> Result<TagDef, TagError> {
        validate_name(name)?;
        self.conn.execute(
            "INSERT INTO tags (name, display_token, created_at) VALUES (?1, ?2, ?3)",
            params![name, display_token(name), now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(TagDef {
            id,
            name: name.to_string(),
            display_token: display_token(name),
            created_at: now,
        })
    }

    pub fn ensure_tag(&mut self, name: &str, now: i64) -> Result<TagDef, TagError> {
        validate_name(name)?;
        if let Some(def) = self.find_tag(name)? {
            return Ok(def);
        }
        self.create_tag(name, now)
    }

    pub fn find_tag(&self, name: &str) -> Result<Option<TagDef>, TagError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, display_token, created_at FROM tags WHERE name = ?1")?;
        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(TagDef {
                id: row.get(0)?,
                name: row.get(1)?,
                display_token: row.get(2)?,
                created_at: row.get(3)?,
            }));
        }
        Ok(None)
    }

    pub fn list_tags(&self) -> Result<Vec<TagDef>, TagError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, display_token, created_at FROM tags ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(TagDef {
                id: row.get(0)?,
                name: row.get(1)?,
                display_token: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_tag(&mut self, name: &str) -> Result<(), TagError> {
        let affected = self
            .conn
            .execute("DELETE FROM tags WHERE name = ?1", params![name])?;
        if affected == 0 {
            return Err(TagError::NotFound(name.to_string()));
        }
        Ok(())
    }

    pub fn tag_paths(
        &mut self,
        paths: &[PathBuf],
        name: &str,
        now: i64,
    ) -> Result<usize, TagError> {
        let def = self.ensure_tag(name, now)?;
        let tx = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO path_tags (path_bytes, tag_id, device_id, inode, updated_at)
                 VALUES (?1, ?2, NULL, NULL, ?3)",
            )?;
            for path in paths {
                changed += stmt.execute(params![path_bytes(path), def.id, now])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn untag_paths(&mut self, paths: &[PathBuf], name: &str) -> Result<usize, TagError> {
        let def = self
            .find_tag(name)?
            .ok_or_else(|| TagError::NotFound(name.to_string()))?;
        let tx = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut stmt =
                tx.prepare("DELETE FROM path_tags WHERE path_bytes = ?1 AND tag_id = ?2")?;
            for path in paths {
                changed += stmt.execute(params![path_bytes(path), def.id])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn tags_for_path(&self, path: &Path) -> Result<Vec<String>, TagError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name FROM tags t
             JOIN path_tags p ON p.tag_id = t.id
             WHERE p.path_bytes = ?1
             ORDER BY t.name",
        )?;
        let rows = stmt.query_map(params![path_bytes(path)], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn tags_for_paths(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<PathBuf, Vec<String>>, TagError> {
        let mut map: HashMap<PathBuf, Vec<String>> = HashMap::new();
        if paths.is_empty() {
            return Ok(map);
        }
        let mut stmt = self.conn.prepare(
            "SELECT p.path_bytes, t.name FROM path_tags p
             JOIN tags t ON t.id = p.tag_id
             ORDER BY t.name",
        )?;
        let wanted: std::collections::HashSet<Vec<u8>> =
            paths.iter().map(|p| path_bytes(p)).collect();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            if !wanted.contains(&bytes) {
                continue;
            }
            let name: String = row.get(1)?;
            map.entry(path_from_bytes(&bytes)).or_default().push(name);
        }
        Ok(map)
    }

    pub fn move_path(&mut self, from: &Path, to: &Path, now: i64) -> Result<usize, TagError> {
        let from_bytes = path_bytes(from);
        let to_bytes = path_bytes(to);
        let mut prefix = from_bytes.clone();
        if !prefix.ends_with(b"/") {
            prefix.push(b'/');
        }
        let tx = self.conn.transaction()?;
        let mut changed = 0usize;
        let mut rewrites: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT path_bytes FROM path_tags")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let bytes: Vec<u8> = row.get(0)?;
                if bytes == from_bytes {
                    rewrites.push((bytes, to_bytes.clone()));
                } else if bytes.starts_with(&prefix) {
                    let suffix = &bytes[prefix.len()..];
                    let mut new_bytes = to_bytes.clone();
                    new_bytes.push(b'/');
                    new_bytes.extend_from_slice(suffix);
                    rewrites.push((bytes, new_bytes));
                }
            }
        }
        for (old, new) in rewrites {
            tx.execute(
                "UPDATE OR REPLACE path_tags SET path_bytes = ?1, updated_at = ?2 WHERE path_bytes = ?3",
                params![new, now, old],
            )?;
            changed += 1;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn schema_version(&self) -> Result<i64, TagError> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TagStore {
        TagStore::open_in_memory().expect("in-memory store")
    }

    #[test]
    fn migration_sets_version() {
        let s = store();
        assert_eq!(s.schema_version().unwrap(), 1);
    }

    #[test]
    fn create_and_validate_names() {
        let mut s = store();
        assert!(s.create_tag("", 0).is_err());
        assert!(s.create_tag(" bad", 0).is_err());
        assert!(s.create_tag("bad\nname", 0).is_err());
        let def = s.create_tag("src", 100).unwrap();
        assert_eq!(def.name, "src");
        assert_eq!(def.display_token, "src");
        assert!(s.create_tag("src", 100).is_err());
    }

    #[test]
    fn many_to_many() {
        let mut s = store();
        let a = PathBuf::from("/data/a.txt");
        let b = PathBuf::from("/data/b.txt");
        s.tag_paths(std::slice::from_ref(&a), "fav", 1).unwrap();
        s.tag_paths(std::slice::from_ref(&a), "src", 2).unwrap();
        s.tag_paths(std::slice::from_ref(&b), "fav", 3).unwrap();
        assert_eq!(s.tags_for_path(&a).unwrap(), vec!["fav", "src"]);
        assert_eq!(s.tags_for_path(&b).unwrap(), vec!["fav"]);
        let map = s.tags_for_paths(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(map.get(&a).unwrap().len(), 2);
        assert_eq!(map.get(&b).unwrap().len(), 1);
    }

    #[test]
    fn untag_and_delete_tag() {
        let mut s = store();
        let a = PathBuf::from("/data/a.txt");
        s.tag_paths(std::slice::from_ref(&a), "fav", 1).unwrap();
        s.untag_paths(std::slice::from_ref(&a), "fav").unwrap();
        assert!(s.tags_for_path(&a).unwrap().is_empty());
        assert!(s.untag_paths(std::slice::from_ref(&a), "ghost").is_err());
        s.tag_paths(std::slice::from_ref(&a), "fav", 2).unwrap();
        s.delete_tag("fav").unwrap();
        assert!(s.tags_for_path(&a).unwrap().is_empty());
        assert!(s.delete_tag("fav").is_err());
    }

    #[test]
    fn move_path_rewrites_prefix() {
        let mut s = store();
        let dir = PathBuf::from("/data/project");
        let child = PathBuf::from("/data/project/src/main.rs");
        let other = PathBuf::from("/data/other.txt");
        s.tag_paths(std::slice::from_ref(&dir), "src", 1).unwrap();
        s.tag_paths(std::slice::from_ref(&child), "fav", 2).unwrap();
        s.tag_paths(std::slice::from_ref(&other), "keep", 3)
            .unwrap();
        let moved = PathBuf::from("/data/renamed");
        let changed = s.move_path(&dir, &moved, 4).unwrap();
        assert_eq!(changed, 2);
        assert_eq!(s.tags_for_path(&moved).unwrap(), vec!["src"]);
        assert_eq!(
            s.tags_for_path(&PathBuf::from("/data/renamed/src/main.rs"))
                .unwrap(),
            vec!["fav"]
        );
        assert_eq!(s.tags_for_path(&other).unwrap(), vec!["keep"]);
        assert!(s.tags_for_path(&dir).unwrap().is_empty());
    }

    #[test]
    fn non_utf8_paths_roundtrip() {
        use std::os::unix::ffi::OsStrExt;
        let mut s = store();
        let raw = b"/data/bad-\xff-name";
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(raw));
        s.tag_paths(std::slice::from_ref(&path), "fav", 1).unwrap();
        assert_eq!(s.tags_for_path(&path).unwrap(), vec!["fav"]);
        let map = s.tags_for_paths(std::slice::from_ref(&path)).unwrap();
        assert!(map.contains_key(&path));
    }
}
