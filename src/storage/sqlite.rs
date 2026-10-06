//! `rusqlite`-backed [`Storage`] implementation.
//!
//! [`SqliteStorage`] wraps a single [`rusqlite::Connection`].  The connection
//! is not `Send` or `Sync`; callers that need cross-thread access (e.g. the
//! async pipeline) should open the connection inside `spawn_blocking` and keep
//! it on that thread.

use std::{
    collections::HashMap,
    fmt::Write as _,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use rusqlite::{Connection, OpenFlags, params, types::Value};

use crate::{
    error::StorageError,
    types::{
        DirectoryStats, EntryBatch, EntryQuery, FileCategory, FileEntry, FileType, JournalMode,
        ScanMetadata, SortDirection, SortField, TypeStat,
    },
};

use super::Storage;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Schema version embedded in `PRAGMA user_version`.
///
/// Increment this when the schema changes in a backward-incompatible way.
pub const SCHEMA_VERSION: u32 = 1;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS entries (
    id           INTEGER PRIMARY KEY,
    path_bytes   BLOB NOT NULL,
    path_text    TEXT NOT NULL,
    parent_bytes BLOB NOT NULL,
    parent_text  TEXT NOT NULL,
    size         INTEGER NOT NULL,
    allocated    INTEGER NOT NULL,
    file_type    INTEGER NOT NULL,
    mode         INTEGER NOT NULL,
    uid          INTEGER NOT NULL,
    gid          INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    inode        INTEGER NOT NULL,
    device       INTEGER NOT NULL,
    nlink        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS scan_metadata (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    root_path      TEXT NOT NULL,
    started_at     INTEGER NOT NULL,
    duration_ms    INTEGER NOT NULL,
    file_count     INTEGER NOT NULL,
    total_size     INTEGER NOT NULL,
    schema_version INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_entries_path   ON entries(path_text);
CREATE INDEX IF NOT EXISTS idx_entries_parent ON entries(parent_text);
CREATE INDEX IF NOT EXISTS idx_entries_size   ON entries(size DESC);
CREATE INDEX IF NOT EXISTS idx_entries_type   ON entries(file_type);
";

const INSERT_SQL: &str = "
INSERT INTO entries
    (path_bytes, path_text, parent_bytes, parent_text,
     size, allocated, file_type, mode, uid, gid, mtime, inode, device, nlink)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
";

/// Column list for all SELECT queries, in the order expected by [`row_to_entry`].
const SELECT_COLS: &str = "path_bytes, path_text, parent_bytes, parent_text, \
     size, allocated, file_type, mode, uid, gid, mtime, inode, device, nlink";

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn i64_from_u64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn system_time_to_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn secs_to_system_time(secs: i64) -> SystemTime {
    u64::try_from(secs).map_or(SystemTime::UNIX_EPOCH, |s| {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    })
}

fn entry_parent(entry: &FileEntry) -> &Path {
    entry.path.parent().unwrap_or(entry.path.as_path())
}

fn entry_mtime_secs(entry: &FileEntry) -> i64 {
    system_time_to_secs(entry.mtime)
}

const fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::Size => "allocated",
        SortField::Name => "path_text",
        SortField::Type => "file_type",
        SortField::Modified => "mtime",
    }
}

const fn sort_direction_sql(dir: SortDirection) -> &'static str {
    match dir {
        SortDirection::Ascending => "ASC",
        SortDirection::Descending => "DESC",
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileEntry> {
    let path_bytes: Vec<u8> = row.get(0)?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&path_bytes));

    let size_i64: i64 = row.get(4)?;
    let allocated_i64: i64 = row.get(5)?;
    let file_type_i64: i64 = row.get(6)?;
    let mode_i64: i64 = row.get(7)?;

    #[allow(clippy::cast_sign_loss)]
    let mode = u32::try_from(mode_i64).unwrap_or(0);
    let category = FileCategory::classify(path.extension(), mode);
    let uid_i64: i64 = row.get(8)?;
    let gid_i64: i64 = row.get(9)?;
    let mtime_secs: i64 = row.get(10)?;
    let inode_i64: i64 = row.get(11)?;
    let device_i64: i64 = row.get(12)?;
    let nlink_i64: i64 = row.get(13)?;

    let file_type = FileType::from_discriminant(u32::try_from(file_type_i64).unwrap_or(6))
        .unwrap_or(FileType::Unknown);

    let mtime = secs_to_system_time(mtime_secs);

    Ok(FileEntry {
        path,
        size: u64::try_from(size_i64).unwrap_or(0),
        allocated_size: u64::try_from(allocated_i64).unwrap_or(0),
        file_type,
        category,
        mode,
        uid: u32::try_from(uid_i64).unwrap_or(0),
        gid: u32::try_from(gid_i64).unwrap_or(0),
        mtime,
        inode: u64::try_from(inode_i64).unwrap_or(0),
        device: u64::try_from(device_i64).unwrap_or(0),
        nlink: u64::try_from(nlink_i64).unwrap_or(0),
    })
}

/// Build a `SELECT` SQL string and parameter list from an [`EntryQuery`].
fn build_query_sql(query: &EntryQuery) -> (String, Vec<Value>) {
    let mut conditions: Vec<&'static str> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(ref prefix) = query.path_prefix {
        conditions.push("path_text LIKE ? ESCAPE '\\'");
        // Escape LIKE wildcards in the prefix so that literal `%`, `_`, and `\`
        // characters in paths are not treated as pattern metacharacters.
        let raw = prefix.to_string_lossy();
        let escaped = raw
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        params.push(Value::Text(format!("{escaped}%")));
    }
    if let Some(min) = query.min_size {
        conditions.push("size >= ?");
        params.push(Value::Integer(i64_from_u64(min)));
    }
    if let Some(max) = query.max_size {
        conditions.push("size <= ?");
        params.push(Value::Integer(i64_from_u64(max)));
    }
    if let Some(ft) = query.file_type {
        conditions.push("file_type = ?");
        params.push(Value::Integer(i64::from(ft.as_discriminant())));
    }

    let col = sort_column(query.sort_by);
    let dir = sort_direction_sql(query.sort_direction);

    let mut sql = format!("SELECT {SELECT_COLS} FROM entries");
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    let _ = write!(sql, " ORDER BY {col} {dir}");
    if let Some(limit) = query.limit {
        let _ = write!(sql, " LIMIT {limit}");
    }

    (sql, params)
}

// ---------------------------------------------------------------------------
// SqliteStorage
// ---------------------------------------------------------------------------

/// [`Storage`] implementation backed by a `rusqlite` `Connection`.
///
/// Use [`SqliteStorage::open`] for write-enabled access (scan phase) and
/// [`SqliteStorage::open_readonly`] for read-only access (explore/export).
#[derive(Debug)]
pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Open or create a database at `path`, applying PRAGMAs and the chosen
    /// `journal_mode`, then initialise the schema.
    pub fn open(path: &Path, journal_mode: JournalMode) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(|e| StorageError::Open {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::apply_pragmas(&conn)?;
        Self::apply_journal_mode(&conn, journal_mode)?;
        let mut storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Open an existing database at `path` in read-only mode.
    ///
    /// Returns [`StorageError::IncompatibleSchema`] if the stored `user_version`
    /// does not match [`SCHEMA_VERSION`].
    pub fn open_readonly(path: &Path) -> Result<Self, StorageError> {
        let conn =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| {
                StorageError::Open {
                    path: path.to_path_buf(),
                    source: e,
                }
            })?;
        Self::apply_pragmas(&conn)?;
        let found: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found != SCHEMA_VERSION {
            return Err(StorageError::IncompatibleSchema {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(Self { conn })
    }

    // --- Private helpers ---

    fn apply_pragmas(conn: &Connection) -> Result<(), StorageError> {
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
        conn.pragma_update(None, "busy_timeout", 5000_i64)?;
        Ok(())
    }

    fn apply_journal_mode(conn: &Connection, mode: JournalMode) -> Result<(), StorageError> {
        let mode_str = match mode {
            JournalMode::Wal => "WAL",
            JournalMode::Delete => "DELETE",
        };
        conn.pragma_update(None, "journal_mode", mode_str)?;
        Ok(())
    }

    fn do_insert_entries(&self, batch: &EntryBatch) -> Result<(), StorageError> {
        let mut stmt = self.conn.prepare_cached(INSERT_SQL)?;
        for entry in batch.entries() {
            let parent = entry_parent(entry);
            let parent_bytes = parent.as_os_str().as_encoded_bytes().to_vec();
            let parent_text = parent.to_string_lossy().into_owned();
            stmt.execute(params![
                entry.path.as_os_str().as_encoded_bytes(),
                entry.path.to_string_lossy().as_ref(),
                parent_bytes,
                parent_text,
                i64_from_u64(entry.size),
                i64_from_u64(entry.allocated_size),
                i64::from(entry.file_type.as_discriminant()),
                i64::from(entry.mode),
                i64::from(entry.uid),
                i64::from(entry.gid),
                entry_mtime_secs(entry),
                i64_from_u64(entry.inode),
                i64_from_u64(entry.device),
                i64_from_u64(entry.nlink),
            ])?;
        }
        Ok(())
    }

    fn do_update_sizes(
        &self,
        sizes: &HashMap<PathBuf, DirectoryStats>,
    ) -> Result<(), StorageError> {
        let mut stmt = self
            .conn
            .prepare_cached("UPDATE entries SET size = ?, allocated = ? WHERE path_text = ?")?;
        for (path, stats) in sizes {
            stmt.execute(params![
                i64_from_u64(stats.total_size),
                i64_from_u64(stats.total_allocated),
                path.to_string_lossy().as_ref(),
            ])?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Storage trait implementation
// ---------------------------------------------------------------------------

impl Storage for SqliteStorage {
    fn init_schema(&mut self) -> Result<(), StorageError> {
        self.conn.execute_batch(SCHEMA_SQL)?;
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    fn insert_batch(&self, batch: &EntryBatch) -> Result<(), StorageError> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = self.do_insert_entries(batch);
        if result.is_ok() {
            self.conn.execute_batch("COMMIT")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }

    fn save_scan_metadata(&self, metadata: &ScanMetadata) -> Result<(), StorageError> {
        let root_path = metadata.root.to_string_lossy();
        let started_at = system_time_to_secs(metadata.started_at);
        let duration_ms = metadata
            .completed_at
            .duration_since(metadata.started_at)
            .map_or(0_u64, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        self.conn.execute(
            "INSERT OR REPLACE INTO scan_metadata \
             (id, root_path, started_at, duration_ms, file_count, total_size, schema_version) \
             VALUES (1, ?, ?, ?, ?, ?, ?)",
            params![
                root_path.as_ref(),
                started_at,
                i64_from_u64(duration_ms),
                i64_from_u64(metadata.entry_count),
                i64_from_u64(metadata.total_size),
                SCHEMA_VERSION,
            ],
        )?;
        Ok(())
    }

    fn load_scan_metadata(&self) -> Result<ScanMetadata, StorageError> {
        let (root_path, started_secs, duration_ms_i64, file_count_i64, total_size_i64) = self
            .conn
            .query_row(
                "SELECT root_path, started_at, duration_ms, file_count, total_size \
                 FROM scan_metadata WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(StorageError::from)?;

        let started_at = secs_to_system_time(started_secs);
        let duration_ms = u64::try_from(duration_ms_i64).unwrap_or(0);
        let completed_at = started_at
            .checked_add(Duration::from_millis(duration_ms))
            .unwrap_or(started_at);

        Ok(ScanMetadata {
            root: PathBuf::from(root_path),
            started_at,
            completed_at,
            entry_count: u64::try_from(file_count_i64).unwrap_or(0),
            total_size: u64::try_from(total_size_i64).unwrap_or(0),
            filesystem_types: vec![],
            // Warnings are transient and are not persisted to the database.
            warnings: vec![],
        })
    }

    fn query_entries(&self, query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError> {
        let (sql, raw_params) = build_query_sql(query);
        let mut stmt = self.conn.prepare(&sql)?;
        let mut entries = stmt
            .query_map(rusqlite::params_from_iter(raw_params.iter()), row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if let Some(cat) = query.category {
            entries.retain(|e| e.category == cat);
        }
        Ok(entries)
    }

    fn query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError> {
        let parent_text = path.to_string_lossy();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM entries WHERE parent_text = ?"
        ))?;
        let entries = stmt
            .query_map([parent_text.as_ref()], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    fn query_top_n_by_size(&self, n: usize) -> Result<Vec<FileEntry>, StorageError> {
        let limit = i64_from_u64(n as u64);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM entries ORDER BY allocated DESC LIMIT ?"
        ))?;
        let entries = stmt
            .query_map([limit], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    fn query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {SELECT_COLS} FROM entries"))?;
        let entries = stmt
            .query_map([], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut map: HashMap<FileCategory, TypeStat> = HashMap::new();
        for entry in entries {
            let type_stat = map.entry(entry.category).or_insert_with(|| TypeStat {
                category: entry.category,
                count: 0,
                total_size: 0,
                total_allocated: 0,
            });
            type_stat.count += 1;
            type_stat.total_size = type_stat.total_size.saturating_add(entry.size);
            type_stat.total_allocated = type_stat
                .total_allocated
                .saturating_add(entry.allocated_size);
        }

        let mut result: Vec<TypeStat> = map.into_values().collect();
        result.sort_by_key(|ts| std::cmp::Reverse(ts.total_size));
        Ok(result)
    }

    fn update_directory_sizes(
        &self,
        sizes: &HashMap<PathBuf, DirectoryStats>,
    ) -> Result<(), StorageError> {
        self.conn.execute_batch("BEGIN")?;
        let result = self.do_update_sizes(sizes);
        if result.is_ok() {
            self.conn.execute_batch("COMMIT")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }

    fn finalize_for_export(&self) -> Result<(), StorageError> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        self.conn.pragma_update(None, "journal_mode", "DELETE")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, time::SystemTime};

    use tempfile::tempdir;

    use super::*;
    use crate::types::{EntryQuery, FileCategory, FileType, JournalMode, SortDirection, SortField};

    // -----------------------------------------------------------------------
    // Local test helper
    // -----------------------------------------------------------------------

    fn make_entry(name: &str, size: u64, file_type: FileType) -> FileEntry {
        FileEntry {
            path: PathBuf::from(format!("/{name}")),
            size,
            allocated_size: size,
            file_type,
            category: FileCategory::NoExtension,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0,
        }
    }

    fn make_entry_at(path: &str, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size,
            allocated_size: size,
            file_type: FileType::Regular,
            category: FileCategory::NoExtension,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0,
        }
    }

    fn open_temp() -> (SqliteStorage, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = SqliteStorage::open(&path, JournalMode::Wal).unwrap();
        (storage, dir)
    }

    // -----------------------------------------------------------------------
    // Schema tests
    // -----------------------------------------------------------------------

    #[test]
    fn init_schema_creates_tables() {
        let (storage, _dir) = open_temp();
        let mut stmt = storage
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(tables.contains(&"entries".to_owned()));
        assert!(tables.contains(&"scan_metadata".to_owned()));
    }

    #[test]
    fn init_schema_sets_user_version() {
        let (storage, _dir) = open_temp();
        let version: u32 = storage
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    // -----------------------------------------------------------------------
    // insert_batch tests
    // -----------------------------------------------------------------------

    #[test]
    fn insert_batch_persists_entries() {
        let (storage, _dir) = open_temp();
        let batch = EntryBatch::new(vec![
            make_entry("a", 100, FileType::Regular),
            make_entry("b", 200, FileType::Regular),
            make_entry("c", 300, FileType::Directory),
        ])
        .unwrap();
        storage.insert_batch(&batch).unwrap();

        let count: i64 = storage
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn insert_batch_stores_path_bytes_losslessly() {
        let (storage, _dir) = open_temp();
        let path = PathBuf::from("/lossless/test");
        let entry = FileEntry {
            path: path.clone(),
            size: 1,
            allocated_size: 1,
            file_type: FileType::Regular,
            category: FileCategory::NoExtension,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0,
        };
        storage
            .insert_batch(&EntryBatch::new(vec![entry]).unwrap())
            .unwrap();

        let bytes: Vec<u8> = storage
            .conn
            .query_row("SELECT path_bytes FROM entries", [], |row| row.get(0))
            .unwrap();
        let reconstructed = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
        assert_eq!(reconstructed, path);
    }

    #[test]
    fn insert_batch_stores_path_text_as_lossy() {
        let (storage, _dir) = open_temp();
        let path = PathBuf::from("/text/test");
        let entry = FileEntry {
            path: path.clone(),
            size: 1,
            allocated_size: 1,
            file_type: FileType::Regular,
            category: FileCategory::NoExtension,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0,
        };
        storage
            .insert_batch(&EntryBatch::new(vec![entry]).unwrap())
            .unwrap();

        let text: String = storage
            .conn
            .query_row("SELECT path_text FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(text, path.to_string_lossy());
    }

    // -----------------------------------------------------------------------
    // scan_metadata tests
    // -----------------------------------------------------------------------

    #[test]
    fn save_and_load_scan_metadata_roundtrips() {
        let (storage, _dir) = open_temp();

        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let completed = started + Duration::from_millis(3_500);
        let metadata = ScanMetadata {
            root: PathBuf::from("/home/user"),
            started_at: started,
            completed_at: completed,
            entry_count: 42,
            total_size: 1_234_567,
            filesystem_types: vec!["ext4".to_owned()],
            warnings: vec![],
        };

        storage.save_scan_metadata(&metadata).unwrap();
        let loaded = storage.load_scan_metadata().unwrap();

        assert_eq!(loaded.root, metadata.root);
        assert_eq!(loaded.started_at, metadata.started_at);
        assert_eq!(loaded.entry_count, metadata.entry_count);
        assert_eq!(loaded.total_size, metadata.total_size);
        // completed_at is derived from duration_ms (integer seconds precision)
        let expected_dur = Duration::from_millis(3_500);
        let actual_dur = loaded
            .completed_at
            .duration_since(loaded.started_at)
            .unwrap();
        assert_eq!(actual_dur, expected_dur);
    }

    // -----------------------------------------------------------------------
    // query tests
    // -----------------------------------------------------------------------

    #[test]
    fn query_directory_children_returns_direct_children() {
        let (storage, _dir) = open_temp();
        let batch = EntryBatch::new(vec![
            make_entry_at("/a/b", 10),
            make_entry_at("/a/c", 20),
            make_entry_at("/a/b/d", 30),
        ])
        .unwrap();
        storage.insert_batch(&batch).unwrap();

        let children = storage.query_directory_children(Path::new("/a")).unwrap();
        assert_eq!(children.len(), 2);
        let mut paths: Vec<_> = children.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        assert_eq!(paths[0], PathBuf::from("/a/b"));
        assert_eq!(paths[1], PathBuf::from("/a/c"));
    }

    #[test]
    fn query_top_n_returns_largest_first() {
        let (storage, _dir) = open_temp();
        let batch = EntryBatch::new(vec![
            make_entry("small", 100, FileType::Regular),
            make_entry("large", 500, FileType::Regular),
            make_entry("medium", 200, FileType::Regular),
        ])
        .unwrap();
        storage.insert_batch(&batch).unwrap();

        let top = storage.query_top_n_by_size(2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].allocated_size, 500);
        assert_eq!(top[1].allocated_size, 200);
    }

    #[test]
    fn query_entries_filters_by_type() {
        let (storage, _dir) = open_temp();
        let batch = EntryBatch::new(vec![
            make_entry("file", 100, FileType::Regular),
            make_entry("dir", 0, FileType::Directory),
        ])
        .unwrap();
        storage.insert_batch(&batch).unwrap();

        let entries = storage
            .query_entries(&EntryQuery {
                file_type: Some(FileType::Regular),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_type, FileType::Regular);
    }

    #[test]
    fn query_entries_sorts_by_name() {
        let (storage, _dir) = open_temp();
        let batch = EntryBatch::new(vec![
            make_entry("b", 10, FileType::Regular),
            make_entry("a", 20, FileType::Regular),
            make_entry("c", 30, FileType::Regular),
        ])
        .unwrap();
        storage.insert_batch(&batch).unwrap();

        let entries = storage
            .query_entries(&EntryQuery {
                sort_by: SortField::Name,
                sort_direction: SortDirection::Ascending,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, PathBuf::from("/a"));
        assert_eq!(entries[1].path, PathBuf::from("/b"));
        assert_eq!(entries[2].path, PathBuf::from("/c"));
    }

    #[test]
    fn update_directory_sizes_modifies_in_place() {
        let (storage, _dir) = open_temp();
        let dir_entry = FileEntry {
            path: PathBuf::from("/mydir"),
            size: 0,
            allocated_size: 0,
            file_type: FileType::Directory,
            category: FileCategory::NoExtension,
            inode: 0,
            device: 0,
            nlink: 1,
            uid: 0,
            gid: 0,
            mtime: SystemTime::UNIX_EPOCH,
            mode: 0,
        };
        storage
            .insert_batch(&EntryBatch::new(vec![dir_entry]).unwrap())
            .unwrap();

        let mut sizes = HashMap::new();
        sizes.insert(
            PathBuf::from("/mydir"),
            DirectoryStats {
                path: PathBuf::from("/mydir"),
                total_size: 1_000,
                total_allocated: 1_000,
                child_count: 5,
            },
        );
        storage.update_directory_sizes(&sizes).unwrap();

        let entries = storage.query_entries(&EntryQuery::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 1_000);
        assert_eq!(entries[0].allocated_size, 1_000);
    }

    #[test]
    fn open_rejects_future_schema_version() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("future.db");

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "user_version", 99u32).unwrap();
        }

        let result = SqliteStorage::open_readonly(&db_path);
        assert!(
            matches!(
                result,
                Err(StorageError::IncompatibleSchema {
                    found: 99,
                    expected: 1
                })
            ),
            "expected IncompatibleSchema, got {result:?}"
        );
    }

    #[test]
    fn finalize_switches_journal_mode() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("wal.db");
        let storage = SqliteStorage::open(&db_path, JournalMode::Wal).unwrap();
        storage.finalize_for_export().unwrap();

        let mode: String = storage
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
    }
}
