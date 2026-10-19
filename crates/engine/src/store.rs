//! SQLite persistence for jobs, segment progress and settings.
//!
//! Jobs are stored as JSON documents so the schema can evolve without
//! migrations for every new field; segment progress lives in its own column
//! because it is rewritten every few seconds while downloading.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::model::{Job, JobId, Segment};
use crate::settings::Settings;

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA_VERSION: i64 = 1;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // WAL + NORMAL: durable across application crashes, cheap commits.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS jobs (
                 id TEXT PRIMARY KEY,
                 data TEXT NOT NULL,
                 segments TEXT NOT NULL DEFAULT '[]',
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS torrent_metadata (info_hash TEXT PRIMARY KEY, data BLOB NOT NULL);",
        )?;
        conn.execute("INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', ?1)", params![SCHEMA_VERSION.to_string()])?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn save_torrent(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        self.conn.lock().execute("INSERT OR REPLACE INTO torrent_metadata(info_hash,data) VALUES (?1,?2)", params![hash, bytes])?;
        Ok(())
    }

    pub fn load_torrent(&self, hash: &str) -> Result<Vec<u8>> {
        Ok(self.conn.lock().query_row("SELECT data FROM torrent_metadata WHERE info_hash=?1", [hash], |r| r.get(0))?)
    }

    pub fn load_jobs(&self) -> Result<Vec<(Job, Vec<Segment>)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data, segments FROM jobs ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (data, segs) = row?;
            match serde_json::from_str::<Job>(&data) {
                Ok(job) => {
                    let segs: Vec<Segment> = serde_json::from_str(&segs).unwrap_or_default();
                    out.push((job, segs));
                }
                Err(e) => tracing::warn!("skipping unreadable job row: {e}"),
            }
        }
        Ok(out)
    }

    pub fn save_job(&self, job: &Job) -> Result<()> {
        let data = serde_json::to_string(job).expect("job serializes");
        self.conn.lock().execute(
            "INSERT INTO jobs (id, data, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![job.id.to_string(), data, job.created_at],
        )?;
        Ok(())
    }

    pub fn save_segments(&self, id: &JobId, segs: &[Segment]) -> Result<()> {
        let data = serde_json::to_string(segs).expect("segments serialize");
        self.conn.lock().execute("UPDATE jobs SET segments = ?2 WHERE id = ?1", params![id.to_string(), data])?;
        Ok(())
    }

    /// Saves the job and its progress atomically.
    pub fn save_job_and_segments(&self, job: &Job, segs: &[Segment]) -> Result<()> {
        let data = serde_json::to_string(job).expect("job serializes");
        let segs = serde_json::to_string(segs).expect("segments serialize");
        self.conn.lock().execute(
            "INSERT INTO jobs (id, data, segments, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, segments = excluded.segments",
            params![job.id.to_string(), data, segs, job.created_at],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, id: &JobId) -> Result<()> {
        self.conn.lock().execute("DELETE FROM jobs WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn load_settings(&self) -> Result<Option<Settings>> {
        let conn = self.conn.lock();
        let v: Option<String> = conn.query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get(0)).optional()?;
        Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let data = serde_json::to_string(s).expect("settings serialize");
        self.conn.lock().execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![data],
        )?;
        Ok(())
    }

    /// Arbitrary small key/value data for the app layer.
    pub fn get_value(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    pub fn set_value(&self, key: &str, value: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}
