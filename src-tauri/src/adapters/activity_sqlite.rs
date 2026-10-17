//! `ActivityLogger` backed by SQLite. Summaries are redacted before they are written.

use crate::domain::activity::{redact, ActivityEvent, ActivityKind};
use crate::domain::{AppError, AppResult, ToolId};
use crate::ports::ActivityLogger;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

const MAX_SUMMARY: usize = 500;

pub struct SqliteActivityLogger {
    conn: Mutex<Connection>,
}

impl SqliteActivityLogger {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS activity (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                at      TEXT NOT NULL,
                kind    TEXT NOT NULL,
                tool_id TEXT,
                summary TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

fn kind_to_str(k: ActivityKind) -> AppResult<String> {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| AppError::Database("unserializable activity kind".into()))
}

fn kind_from_str(s: &str) -> AppResult<ActivityKind> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).map_err(|e| AppError::Database(e.to_string()))
}

impl ActivityLogger for SqliteActivityLogger {
    fn record(&self, event: ActivityEvent) -> AppResult<()> {
        let mut summary = redact(&event.summary);
        summary.truncate(summary.floor_char_boundary(MAX_SUMMARY));
        let conn = self.conn.lock().map_err(|_| AppError::Database("lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO activity (at, kind, tool_id, summary) VALUES (?1, ?2, ?3, ?4)",
            params![event.at, kind_to_str(event.kind)?, event.tool_id.as_ref().map(ToolId::as_str), summary],
        )?;
        Ok(())
    }

    fn recent(&self, limit: usize) -> AppResult<Vec<ActivityEvent>> {
        let conn = self.conn.lock().map_err(|_| AppError::Database("lock poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT at, kind, tool_id, summary FROM activity ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (at, kind, tool, summary) = row?;
            out.push(ActivityEvent {
                at,
                kind: kind_from_str(&kind)?,
                tool_id: tool.map(ToolId::new).transpose()?,
                summary,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(summary: &str) -> ActivityEvent {
        ActivityEvent {
            at: "2026-01-01T00:00:00Z".into(),
            kind: ActivityKind::RunStarted,
            tool_id: Some(ToolId::new("nmap").unwrap()),
            summary: summary.into(),
        }
    }

    #[test]
    fn stores_redacted_summaries_newest_first() {
        let log = SqliteActivityLogger::in_memory().unwrap();
        log.record(ev("first")).unwrap();
        log.record(ev("login --password hunter2")).unwrap();
        let recent = log.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].summary, "login --password [REDACTED]");
        assert!(!recent.iter().any(|e| e.summary.contains("hunter2")));
        assert_eq!(recent[1].summary, "first");
    }

    #[test]
    fn truncates_long_summaries() {
        let log = SqliteActivityLogger::in_memory().unwrap();
        log.record(ev(&"é".repeat(2000))).unwrap();
        assert!(log.recent(1).unwrap()[0].summary.len() <= MAX_SUMMARY);
    }
}
