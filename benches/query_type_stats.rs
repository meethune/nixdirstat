//! Benchmark for `query_type_stats` SQL aggregation.

// Benchmark setup code uses expect/unwrap — panicking on infra failure is correct.
#![allow(clippy::expect_used, clippy::cast_possible_wrap)]

use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use nixdirstat::{
    storage::{ReadStorage as _, sqlite::SqliteStorage},
    types::JournalMode,
};

fn create_and_populate(path: &Path, n: usize) {
    let storage = SqliteStorage::open(path, JournalMode::Wal).expect("open");
    drop(storage);

    let conn = rusqlite::Connection::open(path).expect("reopen");
    let categories: &[u32] = &[0, 1, 2, 3, 4, 5, 6, 7, 8];
    conn.execute_batch("BEGIN IMMEDIATE").expect("begin");
    let mut stmt = conn
        .prepare_cached(
            "INSERT INTO entries \
             (path_bytes, path_text, parent_bytes, parent_text, \
              size, allocated, file_type, mode, uid, gid, mtime, \
              inode, device, nlink, category) \
             VALUES (?, ?, ?, ?, ?, ?, 0, 420, 1000, 1000, 0, ?, 1, 1, ?)",
        )
        .expect("prepare");
    for i in 0..n {
        let path = format!("/bench/dir{}/file{i}.ext", i / 100);
        let parent = format!("/bench/dir{}", i / 100);
        let size = (i as i64 + 1) * 1024;
        let cat = categories[i % categories.len()];
        stmt.execute(rusqlite::params![
            path.as_bytes(),
            &path,
            parent.as_bytes(),
            &parent,
            size,
            size,
            i as i64,
            cat,
        ])
        .expect("insert");
    }
    conn.execute_batch("COMMIT").expect("commit");
}

fn bench_query_type_stats(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bench.db");
    create_and_populate(&db_path, 100_000);
    let storage = SqliteStorage::open_readonly(&db_path).expect("open_readonly");

    c.bench_function("query_type_stats_100k", |b| {
        b.iter(|| {
            let stats = storage.query_type_stats().expect("query");
            std::hint::black_box(stats);
        });
    });
}

criterion_group!(benches, bench_query_type_stats);
criterion_main!(benches);
