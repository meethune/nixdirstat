//! Storage layer for NixDirStat.
//!
//! Defines the [`ReadStorage`] and [`WriteStorage`] traits that all persistence
//! backends must implement, and re-exports the production [`sqlite`] implementation.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    error::StorageError,
    types::{DirectoryStats, EntryBatch, EntryQuery, FileEntry, ScanMetadata, TypeStat},
};

pub mod sqlite;

/// Read-only persistence backend for querying scan results.
///
/// All methods take `&self` (shared reference) because the underlying
/// connection uses interior mutability for cached statement preparation.
pub trait ReadStorage {
    /// Read the scan summary metadata stored by a previous scan.
    fn load_scan_metadata(&self) -> Result<ScanMetadata, StorageError>;

    /// Return file entries matching the given query parameters.
    fn query_entries(&self, query: &EntryQuery) -> Result<Vec<FileEntry>, StorageError>;

    /// Return all direct children of `path` (entries whose parent equals `path`).
    fn query_directory_children(&self, path: &Path) -> Result<Vec<FileEntry>, StorageError>;

    /// Return the `n` largest entries by allocated size, largest first.
    fn query_top_n_by_size(&self, n: usize) -> Result<Vec<FileEntry>, StorageError>;

    /// Return per-category size and count aggregations across all stored entries.
    fn query_type_stats(&self) -> Result<Vec<TypeStat>, StorageError>;
}

/// Read-write persistence backend for scan ingestion.
///
/// Extends [`ReadStorage`] with mutation methods used during the scan pipeline.
pub trait WriteStorage: ReadStorage {
    /// Create the database schema (tables, indexes, `user_version`).
    ///
    /// Uses `CREATE TABLE IF NOT EXISTS` so that calling this method on an
    /// already-initialised database is safe and idempotent.
    fn init_schema(&mut self) -> Result<(), StorageError>;

    /// Persist a batch of file entries inside a single `BEGIN IMMEDIATE`
    /// transaction.
    fn insert_batch(&self, batch: &EntryBatch) -> Result<(), StorageError>;

    /// Write scan summary metadata; replaces any previously stored record.
    fn save_scan_metadata(&self, metadata: &ScanMetadata) -> Result<(), StorageError>;

    /// Update the `size` and `allocated` columns for the given directory paths
    /// in a single transaction.  Used by the analyzer after bottom-up
    /// aggregation.
    fn update_directory_sizes(
        &self,
        sizes: &HashMap<PathBuf, DirectoryStats>,
    ) -> Result<(), StorageError>;
}
