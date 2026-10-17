//! SQLite persistence for jobs, job output and installation records.
//!
//! A partial unique index guarantees at most one *active* job per tool even across processes, so
//! duplicate installation requests cannot corrupt an installation.

use crate::domain::activity::redact;
use crate::domain::jobs::{InstallationRecord, Job, JobError, JobKind, JobPhase};
use crate::domain::process::OutputStream;
use crate::domain::{AppError, AppResult, OperationId, ToolId};
use crate::ports::JobStore;
use rusqlite::{params, Connection, ErrorCode, Row};
use std::path::Path;
use std::sync::Mutex;

const MAX_LINE: usize = 2000;
const TRIM_ABOVE: i64 = 3000;
const KEEP_LINES: i64 = 2000;

pub struct SqliteJobStore {
    conn: Mutex<Connection>,
}

fn active_list() -> String {
    JobPhase::ACTIVE.iter().map(|p| format!("'{}'", p.as_str())).collect::<Vec<_>>().join(",")
}

impl SqliteJobStore {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Self::init(conn)
    }

    pub fn in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                phase TEXT NOT NULL,
                message TEXT NOT NULL,
                percent INTEGER,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                finished_at TEXT,
                exit_code INTEGER,
                allow_elevation INTEGER NOT NULL,
                error_json TEXT
            );
            CREATE UNIQUE INDEX IF NOT EXISTS one_active_job_per_tool ON jobs(tool_id) WHERE phase IN ({});
            CREATE TABLE IF NOT EXISTS job_output (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL,
                stream TEXT NOT NULL,
                text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS job_output_job ON job_output(job_id);
            CREATE TABLE IF NOT EXISTS installations (
                tool_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );",
            active_list()
        ))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| AppError::Database("store lock poisoned".into()))
    }
}

fn map_err(e: rusqlite::Error, tool: &ToolId) -> AppError {
    if let rusqlite::Error::SqliteFailure(f, _) = &e {
        if f.code == ErrorCode::ConstraintViolation {
            return AppError::Busy(format!("another operation on '{tool}' is already in progress"));
        }
    }
    e.into()
}

fn json<T: serde::Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_string(v).map_err(|e| AppError::Database(e.to_string()))
}

fn unjson<T: serde::de::DeserializeOwned>(s: &str) -> AppResult<T> {
    serde_json::from_str(s).map_err(|e| AppError::Database(format!("corrupt stored value: {e}")))
}

fn enum_str<T: serde::Serialize>(v: &T) -> AppResult<String> {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(AppError::Database("unserializable enum".into())),
    }
}

fn job_from_row(r: &Row<'_>) -> AppResult<Job> {
    let get = |i: usize| -> AppResult<String> { r.get::<_, String>(i).map_err(AppError::from) };
    let id = uuid::Uuid::parse_str(&get(0)?).map_err(|e| AppError::Database(e.to_string()))?;
    let error: Option<String> = r.get(11)?;
    Ok(Job {
        id: OperationId(id),
        tool_id: ToolId::new(get(1)?)?,
        kind: unjson::<JobKind>(&format!("\"{}\"", get(2)?))?,
        phase: JobPhase::parse(&get(3)?)?,
        message: get(4)?,
        percent: r.get::<_, Option<i64>>(5)?.map(|p| p.clamp(0, 100) as u8),
        started_at: get(6)?,
        updated_at: get(7)?,
        finished_at: r.get(8)?,
        exit_code: r.get::<_, Option<i64>>(9)?.map(|c| c as i32),
        allow_elevation: r.get::<_, i64>(10)? != 0,
        error: error.map(|s| unjson::<JobError>(&s)).transpose()?,
    })
}

const JOB_COLS: &str = "id, tool_id, kind, phase, message, percent, started_at, updated_at, finished_at, exit_code, allow_elevation, error_json";

impl JobStore for SqliteJobStore {
    fn create_job(&self, job: &Job) -> AppResult<()> {
        let conn = self.conn()?;
        conn.execute(
            &format!("INSERT INTO jobs ({JOB_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"),
            params![
                job.id.to_string(),
                job.tool_id.as_str(),
                enum_str(&job.kind)?,
                job.phase.as_str(),
                job.message,
                job.percent.map(i64::from),
                job.started_at,
                job.updated_at,
                job.finished_at,
                job.exit_code.map(i64::from),
                i64::from(job.allow_elevation),
                job.error.as_ref().map(json).transpose()?,
            ],
        )
        .map_err(|e| map_err(e, &job.tool_id))?;
        Ok(())
    }

    fn update_job(&self, job: &Job) -> AppResult<()> {
        let conn = self.conn()?;
        let n = conn
            .execute(
                "UPDATE jobs SET phase=?2, message=?3, percent=?4, updated_at=?5, finished_at=?6, exit_code=?7, error_json=?8 WHERE id=?1",
                params![
                    job.id.to_string(),
                    job.phase.as_str(),
                    job.message,
                    job.percent.map(i64::from),
                    job.updated_at,
                    job.finished_at,
                    job.exit_code.map(i64::from),
                    job.error.as_ref().map(json).transpose()?,
                ],
            )
            .map_err(|e| map_err(e, &job.tool_id))?;
        if n == 0 {
            return Err(AppError::NotFound(format!("job '{}'", job.id)));
        }
        Ok(())
    }

    fn get_job(&self, id: OperationId) -> AppResult<Job> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {JOB_COLS} FROM jobs WHERE id=?1"))?;
        let mut rows = stmt.query(params![id.to_string()])?;
        match rows.next()? {
            Some(r) => job_from_row(r),
            None => Err(AppError::NotFound(format!("job '{id}'"))),
        }
    }

    fn list_jobs(&self, limit: usize) -> AppResult<Vec<Job>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {JOB_COLS} FROM jobs ORDER BY rowid DESC LIMIT ?1"))?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(job_from_row(r)?);
        }
        Ok(out)
    }

    fn active_job_for(&self, tool: &ToolId) -> AppResult<Option<Job>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {JOB_COLS} FROM jobs WHERE tool_id=?1 AND phase IN ({}) LIMIT 1", active_list()))?;
        let mut rows = stmt.query(params![tool.as_str()])?;
        rows.next()?.map(job_from_row).transpose()
    }

    fn append_output(&self, id: OperationId, stream: OutputStream, text: &str) -> AppResult<()> {
        let mut clean = redact(text);
        clean.truncate(clean.floor_char_boundary(MAX_LINE));
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO job_output (job_id, stream, text) VALUES (?1, ?2, ?3)",
            params![id.to_string(), enum_str(&stream)?, clean],
        )?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM job_output WHERE job_id=?1", params![id.to_string()], |r| r.get(0))?;
        if count > TRIM_ABOVE {
            conn.execute(
                "DELETE FROM job_output WHERE id IN (SELECT id FROM job_output WHERE job_id=?1 ORDER BY id ASC LIMIT ?2)",
                params![id.to_string(), count - KEEP_LINES],
            )?;
        }
        Ok(())
    }

    fn job_output(&self, id: OperationId, limit: usize) -> AppResult<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT text FROM (SELECT id, text FROM job_output WHERE job_id=?1 ORDER BY id DESC LIMIT ?2) ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![id.to_string(), limit as i64], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn interrupt_active(&self, now: &str) -> AppResult<Vec<Job>> {
        let active: Vec<Job> = {
            let conn = self.conn()?;
            let mut stmt = conn.prepare(&format!("SELECT {JOB_COLS} FROM jobs WHERE phase IN ({})", active_list()))?;
            let mut rows = stmt.query([])?;
            let mut v = Vec::new();
            while let Some(r) = rows.next()? {
                v.push(job_from_row(r)?);
            }
            v
        };
        let mut out = Vec::new();
        for mut job in active {
            // Jobs that had not touched the system yet are simply cancelled; the rest are failed
            // because their real effect is unknown until the tool is re-checked.
            let touched = matches!(job.phase, JobPhase::Installing | JobPhase::Verifying | JobPhase::Uninstalling | JobPhase::Cancelling | JobPhase::RollingBack);
            if touched {
                job.phase = JobPhase::Failed;
                job.error = Some(JobError {
                    code: "interrupted".into(),
                    message: "Vixalune was closed while this operation was running.".into(),
                    recovery: "Use Verify to check the tool's current state, then Retry or Uninstall as needed.".into(),
                    detail: Some(format!("interrupted during phase '{}'", job.phase.as_str())),
                });
            } else {
                job.phase = JobPhase::Cancelled;
            }
            job.message = if touched { "Interrupted by application restart".into() } else { "Cancelled by application restart".into() };
            job.updated_at = now.to_string();
            job.finished_at = Some(now.to_string());
            self.update_job(&job)?;
            out.push(job);
        }
        Ok(out)
    }

    fn put_installation(&self, record: &InstallationRecord) -> AppResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO installations (tool_id, record_json) VALUES (?1, ?2)
             ON CONFLICT(tool_id) DO UPDATE SET record_json=excluded.record_json",
            params![record.tool_id.as_str(), json(record)?],
        )?;
        Ok(())
    }

    fn get_installation(&self, tool: &ToolId) -> AppResult<Option<InstallationRecord>> {
        let conn = self.conn()?;
        let s: Option<String> = conn
            .query_row("SELECT record_json FROM installations WHERE tool_id=?1", params![tool.as_str()], |r| r.get(0))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })?;
        s.map(|s| unjson(&s)).transpose()
    }

    fn list_installations(&self) -> AppResult<Vec<InstallationRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT record_json FROM installations ORDER BY tool_id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for s in rows {
            out.push(unjson(&s?)?);
        }
        Ok(out)
    }

    fn delete_installation(&self, tool: &ToolId) -> AppResult<()> {
        self.conn()?.execute("DELETE FROM installations WHERE tool_id=?1", params![tool.as_str()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::install_state::InstallState;
    use crate::domain::manifest::InstallMethod;

    fn job(tool: &str, phase: JobPhase) -> Job {
        Job {
            id: OperationId::new(),
            tool_id: ToolId::new(tool).unwrap(),
            kind: JobKind::Install,
            phase,
            message: "m".into(),
            percent: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            exit_code: None,
            allow_elevation: false,
            error: None,
        }
    }

    #[test]
    fn round_trips_jobs_and_orders_newest_first() {
        let s = SqliteJobStore::in_memory().unwrap();
        let a = job("a", JobPhase::Succeeded);
        let mut b = job("b", JobPhase::Queued);
        s.create_job(&a).unwrap();
        s.create_job(&b).unwrap();
        b.phase = JobPhase::Failed;
        b.error = Some(JobError { code: "x".into(), message: "bad".into(), recovery: "fix".into(), detail: Some("d".into()) });
        b.exit_code = Some(5);
        s.update_job(&b).unwrap();
        let list = s.list_jobs(10).unwrap();
        assert_eq!(list[0], b);
        assert_eq!(list[1], a);
        assert_eq!(s.get_job(a.id).unwrap(), a);
        assert!(s.get_job(OperationId::new()).is_err());
    }

    #[test]
    fn only_one_active_job_per_tool() {
        let s = SqliteJobStore::in_memory().unwrap();
        s.create_job(&job("a", JobPhase::Installing)).unwrap();
        let err = s.create_job(&job("a", JobPhase::Queued)).unwrap_err();
        assert!(matches!(err, AppError::Busy(_)), "{err:?}");
        // Other tools and finished jobs are not blocked.
        s.create_job(&job("b", JobPhase::Queued)).unwrap();
        s.create_job(&job("a", JobPhase::Failed)).unwrap();
        assert!(s.active_job_for(&ToolId::new("a").unwrap()).unwrap().is_some());
        assert!(s.active_job_for(&ToolId::new("zzz").unwrap()).unwrap().is_none());
    }

    #[test]
    fn finishing_a_job_frees_the_tool() {
        let s = SqliteJobStore::in_memory().unwrap();
        let mut j = job("a", JobPhase::Installing);
        s.create_job(&j).unwrap();
        j.phase = JobPhase::Succeeded;
        s.update_job(&j).unwrap();
        s.create_job(&job("a", JobPhase::Queued)).unwrap();
    }

    #[test]
    fn output_is_redacted_ordered_and_bounded() {
        let s = SqliteJobStore::in_memory().unwrap();
        let j = job("a", JobPhase::Installing);
        s.create_job(&j).unwrap();
        s.append_output(j.id, OutputStream::Stdout, "first").unwrap();
        s.append_output(j.id, OutputStream::Stderr, "login --password hunter2 --host x").unwrap();
        s.append_output(j.id, OutputStream::Stdout, &"é".repeat(5000)).unwrap();
        let out = s.job_output(j.id, 100).unwrap();
        assert_eq!(out[0], "first");
        assert_eq!(out[1], "login --password [REDACTED] --host x");
        assert!(out[2].len() <= MAX_LINE);
        assert!(!out.join("\n").contains("hunter2"));
        for i in 0..3100 {
            s.append_output(j.id, OutputStream::Stdout, &format!("line {i}")).unwrap();
        }
        let all = s.job_output(j.id, 100_000).unwrap();
        assert!(all.len() <= TRIM_ABOVE as usize);
        assert_eq!(all.last().unwrap(), "line 3099");
        assert_eq!(s.job_output(j.id, 2).unwrap().len(), 2);
    }

    #[test]
    fn restart_recovery_marks_interrupted_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let (installing, awaiting, done);
        {
            let s = SqliteJobStore::open(&path).unwrap();
            installing = job("a", JobPhase::Installing);
            awaiting = job("b", JobPhase::AwaitingConsent);
            done = job("c", JobPhase::Succeeded);
            for j in [&installing, &awaiting, &done] {
                s.create_job(j).unwrap();
            }
            s.append_output(installing.id, OutputStream::Stdout, "downloading").unwrap();
        } // connection closed: simulates the app exiting
        let s = SqliteJobStore::open(&path).unwrap();
        let changed = s.interrupt_active("2026-02-02T00:00:00Z").unwrap();
        assert_eq!(changed.len(), 2);
        let a = s.get_job(installing.id).unwrap();
        assert_eq!(a.phase, JobPhase::Failed);
        assert_eq!(a.error.as_ref().unwrap().code, "interrupted");
        assert_eq!(a.finished_at.as_deref(), Some("2026-02-02T00:00:00Z"));
        assert_eq!(s.get_job(awaiting.id).unwrap().phase, JobPhase::Cancelled);
        assert_eq!(s.get_job(done.id).unwrap().phase, JobPhase::Succeeded);
        assert_eq!(s.job_output(installing.id, 10).unwrap(), vec!["downloading"]);
        // The tool is free again.
        s.create_job(&job("a", JobPhase::Queued)).unwrap();
    }

    #[test]
    fn installation_records_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let tool = ToolId::new("nmap").unwrap();
        let rec = InstallationRecord {
            tool_id: tool.clone(),
            state: InstallState::Installed,
            version: Some("7.80".into()),
            location: Some("C:\\x".into()),
            adapter: InstallMethod::Winget,
            manifest_version: "1.0.0".into(),
            installed_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_verification: None,
        };
        SqliteJobStore::open(&path).unwrap().put_installation(&rec).unwrap();
        let s = SqliteJobStore::open(&path).unwrap();
        assert_eq!(s.get_installation(&tool).unwrap(), Some(rec.clone()));
        assert_eq!(s.list_installations().unwrap(), vec![rec]);
        s.delete_installation(&tool).unwrap();
        assert_eq!(s.get_installation(&tool).unwrap(), None);
    }
}
