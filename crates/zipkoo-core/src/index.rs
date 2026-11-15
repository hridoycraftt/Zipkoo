//! Search inside every archive at once. A small SQLite index (in the app's data folder) stores
//! each archive's file list; archives are re-read only when their size or date changes.
use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct Index {
    db: Connection,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Hit {
    pub archive: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct RefreshReport {
    pub indexed: usize,
    pub unchanged: usize,
    pub failed: Vec<(String, String)>,
    pub removed: usize,
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::Damaged(format!("search index: {e}"))
}

fn stamp(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((m.len() as i64, t))
}

const EXTS: &[&str] = &[
    "zip", "zipx", "7z", "rar", "tar", "tgz", "gz", "zst", "tzst", "bz2", "tbz2", "xz", "txz",
    "iso", "cab", "lzh", "lha",
];

/// Archives under `dir` (not following links). Split parts count once (.001).
pub fn find_archives(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.') || e.depth() == 0)
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            n.ends_with(".001") || EXTS.iter().any(|x| n.ends_with(&format!(".{x}")))
        })
        .collect()
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let db = Connection::open(path).map_err(db_err)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS archives(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, size INTEGER, mtime INTEGER);
             CREATE TABLE IF NOT EXISTS entries(archive INTEGER NOT NULL REFERENCES archives(id) ON DELETE CASCADE, name TEXT NOT NULL, lname TEXT NOT NULL, size INTEGER);
             CREATE INDEX IF NOT EXISTS entries_archive ON entries(archive);
             PRAGMA foreign_keys=ON;",
        )
        .map_err(db_err)?;
        Ok(Self { db })
    }

    /// Index (or re-index) these archives. Locked archives with hidden names are skipped.
    pub fn refresh(&mut self, archives: &[PathBuf]) -> Result<RefreshReport> {
        let mut r = RefreshReport::default();
        for a in archives {
            let key = a.to_string_lossy().into_owned();
            let Some((size, mtime)) = stamp(a) else {
                continue;
            };
            let known: Option<(i64, i64)> = self
                .db
                .query_row(
                    "SELECT size, mtime FROM archives WHERE path=?1",
                    [&key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            if known == Some((size, mtime)) {
                r.unchanged += 1;
                continue;
            }
            match crate::inspect::inspect(a, None) {
                Ok(info) => {
                    let tx = self.db.transaction().map_err(db_err)?;
                    tx.execute("DELETE FROM archives WHERE path=?1", [&key])
                        .map_err(db_err)?;
                    tx.execute(
                        "INSERT INTO archives(path,size,mtime) VALUES(?1,?2,?3)",
                        params![key, size, mtime],
                    )
                    .map_err(db_err)?;
                    let id = tx.last_insert_rowid();
                    {
                        let mut st = tx
                            .prepare(
                                "INSERT INTO entries(archive,name,lname,size) VALUES(?1,?2,?3,?4)",
                            )
                            .map_err(db_err)?;
                        for e in info
                            .entries
                            .iter()
                            .filter(|e| !e.is_dir && !e.name.starts_with(".zipkoo/"))
                        {
                            st.execute(params![id, e.name, e.name.to_lowercase(), e.size as i64])
                                .map_err(db_err)?;
                        }
                    }
                    tx.commit().map_err(db_err)?;
                    r.indexed += 1;
                }
                Err(e) => r.failed.push((key, e.to_string())),
            }
        }
        Ok(r)
    }

    /// Forget archives that no longer exist.
    pub fn prune(&mut self) -> Result<usize> {
        let paths: Vec<String> = {
            let mut st = self
                .db
                .prepare("SELECT path FROM archives")
                .map_err(db_err)?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(db_err)?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut n = 0;
        for p in paths.iter().filter(|p| !Path::new(p).exists()) {
            n += self
                .db
                .execute("DELETE FROM archives WHERE path=?1", [p])
                .map_err(db_err)?;
        }
        Ok(n)
    }

    /// Every word must appear in the file's path inside the archive (case-insensitive).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = String::from("SELECT a.path, e.name, e.size FROM entries e JOIN archives a ON a.id=e.archive WHERE 1=1");
        for i in 0..words.len() {
            sql.push_str(&format!(" AND instr(e.lname, ?{}) > 0", i + 1));
        }
        sql.push_str(&format!(" ORDER BY length(e.name) LIMIT {limit}"));
        let mut st = self.db.prepare(&sql).map_err(db_err)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(words.iter()), |r| {
                Ok(Hit {
                    archive: r.get(0)?,
                    name: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn stats(&self) -> Result<(u64, u64)> {
        let a: i64 = self
            .db
            .query_row("SELECT count(*) FROM archives", [], |r| r.get(0))
            .map_err(db_err)?;
        let e: i64 = self
            .db
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .map_err(db_err)?;
        Ok((a as u64, e as u64))
    }
}
