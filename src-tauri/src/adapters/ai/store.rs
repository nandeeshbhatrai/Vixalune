//! SQLite persistence for conversations, messages and non-secret provider settings. No credential is ever written here: the
//! schema has no column for one, and provider configuration is a closed struct that cannot carry a key.

use crate::domain::ai::*;
use crate::domain::{AppError, AppResult};
use crate::ports::AiStore;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct SqliteAiStore {
    conn: Mutex<Connection>,
}

fn json<T: serde::Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_string(v).map_err(|e| AppError::Database(e.to_string()))
}

fn unjson<T: serde::de::DeserializeOwned>(s: &str) -> AppResult<T> {
    serde_json::from_str(s).map_err(|e| AppError::Database(format!("corrupt stored value: {e}")))
}

fn enum_str<T: serde::Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_value(v).ok().and_then(|x| x.as_str().map(String::from)).ok_or_else(|| AppError::Database("unserializable value".into()))
}

fn enum_from<T: serde::de::DeserializeOwned>(s: &str) -> AppResult<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| AppError::Database(e.to_string()))
}

impl SqliteAiStore {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Self::init(conn)
    }

    pub fn in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ai_providers (
                provider TEXT PRIMARY KEY,
                config_json TEXT NOT NULL,
                status_json TEXT NOT NULL,
                key_hint TEXT
            );
            CREATE TABLE IF NOT EXISTS ai_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS ai_conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS ai_conversations_updated ON ai_conversations(updated_at);
            CREATE TABLE IF NOT EXISTS ai_messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL,
                error_code TEXT,
                error_message TEXT,
                provider TEXT,
                model TEXT,
                usage_json TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS ai_messages_conv ON ai_messages(conversation_id, seq);
            CREATE TABLE IF NOT EXISTS ai_turns (
                message_id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                turn_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS ai_turns_conv ON ai_turns(conversation_id);
            CREATE TRIGGER IF NOT EXISTS ai_turns_cleanup AFTER DELETE ON ai_messages
            BEGIN
                DELETE FROM ai_turns WHERE message_id = old.id;
            END;",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| AppError::Database("the AI database lock was poisoned".into()))
    }

    fn ensure_row(conn: &Connection, provider: &str) -> AppResult<()> {
        conn.execute(
            "INSERT OR IGNORE INTO ai_providers (provider, config_json, status_json) VALUES (?1, ?2, ?3)",
            params![provider, json(&ProviderConfig::new(provider))?, json(&ProviderStatus::default())?],
        )?;
        Ok(())
    }
}

fn message_from_row(r: &rusqlite::Row<'_>) -> AppResult<StoredMessage> {
    let get = |i: usize| -> rusqlite::Result<String> { r.get(i) };
    let opt = |i: usize| -> rusqlite::Result<Option<String>> { r.get(i) };
    Ok(StoredMessage {
        id: get(0)?,
        conversation_id: get(1)?,
        seq: r.get(2)?,
        role: enum_from(&get(3)?)?,
        content: get(4)?,
        status: enum_from(&get(5)?)?,
        error_code: opt(6)?,
        error_message: opt(7)?,
        provider: opt(8)?,
        model: opt(9)?,
        usage: opt(10)?.map(|s| unjson(&s)).transpose()?,
        created_at: get(11)?,
    })
}

const MSG_COLS: &str = "id, conversation_id, seq, role, content, status, error_code, error_message, provider, model, usage_json, created_at";

impl AiStore for SqliteAiStore {
    fn get_config(&self, provider: &str) -> AppResult<Option<ProviderConfig>> {
        let conn = self.conn()?;
        let s: Option<String> = conn.query_row("SELECT config_json FROM ai_providers WHERE provider=?1", params![provider], |r| r.get(0)).optional()?;
        s.map(|s| unjson(&s)).transpose()
    }

    fn put_config(&self, config: &ProviderConfig) -> AppResult<()> {
        let conn = self.conn()?;
        Self::ensure_row(&conn, &config.provider)?;
        conn.execute("UPDATE ai_providers SET config_json=?2 WHERE provider=?1", params![config.provider, json(config)?])?;
        Ok(())
    }

    fn delete_config(&self, provider: &str) -> AppResult<()> {
        self.conn()?.execute("DELETE FROM ai_providers WHERE provider=?1", params![provider])?;
        Ok(())
    }

    fn get_status(&self, provider: &str) -> AppResult<ProviderStatus> {
        let conn = self.conn()?;
        let s: Option<String> = conn.query_row("SELECT status_json FROM ai_providers WHERE provider=?1", params![provider], |r| r.get(0)).optional()?;
        match s {
            Some(s) => unjson(&s),
            None => Ok(ProviderStatus::default()),
        }
    }

    fn put_status(&self, provider: &str, status: &ProviderStatus) -> AppResult<()> {
        let conn = self.conn()?;
        Self::ensure_row(&conn, provider)?;
        conn.execute("UPDATE ai_providers SET status_json=?2 WHERE provider=?1", params![provider, json(status)?])?;
        Ok(())
    }

    fn get_key_hint(&self, provider: &str) -> AppResult<Option<String>> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT key_hint FROM ai_providers WHERE provider=?1", params![provider], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
    }

    fn put_key_hint(&self, provider: &str, hint: Option<&str>) -> AppResult<()> {
        let conn = self.conn()?;
        Self::ensure_row(&conn, provider)?;
        conn.execute("UPDATE ai_providers SET key_hint=?2 WHERE provider=?1", params![provider, hint])?;
        Ok(())
    }

    fn get_settings(&self) -> AppResult<AiSettings> {
        let conn = self.conn()?;
        let v: Option<String> = conn.query_row("SELECT value FROM ai_meta WHERE key='settings'", [], |r| r.get(0)).optional()?;
        Ok(v.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    fn put_settings(&self, s: &AiSettings) -> AppResult<()> {
        s.validate()?;
        self.conn()?.execute("INSERT INTO ai_meta (key, value) VALUES ('settings', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![json(s)?])?;
        Ok(())
    }

    fn active_provider(&self) -> AppResult<Option<String>> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT value FROM ai_meta WHERE key='active_provider'", [], |r| r.get(0)).optional()?)
    }

    fn set_active_provider(&self, provider: Option<&str>) -> AppResult<()> {
        let conn = self.conn()?;
        match provider {
            Some(p) => conn.execute("INSERT INTO ai_meta (key, value) VALUES ('active_provider', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![p])?,
            None => conn.execute("DELETE FROM ai_meta WHERE key='active_provider'", [])?,
        };
        Ok(())
    }

    fn create_conversation(&self, c: &Conversation) -> AppResult<()> {
        self.conn()?.execute(
            "INSERT INTO ai_conversations (id, title, provider, model, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![c.id, c.title, c.provider, c.model, c.created_at, c.updated_at],
        )?;
        Ok(())
    }

    fn update_conversation(&self, c: &Conversation) -> AppResult<()> {
        let n = self.conn()?.execute("UPDATE ai_conversations SET title=?2, provider=?3, model=?4, updated_at=?5 WHERE id=?1", params![c.id, c.title, c.provider, c.model, c.updated_at])?;
        if n == 0 {
            return Err(AppError::NotFound(format!("conversation '{}'", c.id)));
        }
        Ok(())
    }

    fn get_conversation(&self, id: &str) -> AppResult<Conversation> {
        let conn = self.conn()?;
        conn.query_row("SELECT id, title, provider, model, created_at, updated_at FROM ai_conversations WHERE id=?1", params![id], |r| {
            Ok(Conversation { id: r.get(0)?, title: r.get(1)?, provider: r.get(2)?, model: r.get(3)?, created_at: r.get(4)?, updated_at: r.get(5)? })
        })
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("conversation '{id}'")))
    }

    fn list_conversations(&self, limit: usize) -> AppResult<Vec<Conversation>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT id, title, provider, model, created_at, updated_at FROM ai_conversations ORDER BY updated_at DESC, rowid DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| Ok(Conversation { id: r.get(0)?, title: r.get(1)?, provider: r.get(2)?, model: r.get(3)?, created_at: r.get(4)?, updated_at: r.get(5)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_conversation(&self, id: &str) -> AppResult<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM ai_messages WHERE conversation_id=?1", params![id])?;
        conn.execute("DELETE FROM ai_conversations WHERE id=?1", params![id])?;
        Ok(())
    }

    fn delete_all_conversations(&self) -> AppResult<usize> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM ai_messages", [])?;
        Ok(conn.execute("DELETE FROM ai_conversations", [])?)
    }

    fn append_message(&self, m: &StoredMessage) -> AppResult<StoredMessage> {
        let conn = self.conn()?;
        let seq: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM ai_messages WHERE conversation_id=?1", params![m.conversation_id], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO ai_messages (id, conversation_id, seq, role, content, status, error_code, error_message, provider, model, usage_json, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![m.id, m.conversation_id, seq, enum_str(&m.role)?, m.content, enum_str(&m.status)?, m.error_code, m.error_message, m.provider, m.model, m.usage.as_ref().map(json).transpose()?, m.created_at],
        )?;
        let mut out = m.clone();
        out.seq = seq;
        Ok(out)
    }

    fn update_message(&self, m: &StoredMessage) -> AppResult<()> {
        let n = self.conn()?.execute(
            "UPDATE ai_messages SET content=?2, status=?3, error_code=?4, error_message=?5, provider=?6, model=?7, usage_json=?8 WHERE id=?1",
            params![m.id, m.content, enum_str(&m.status)?, m.error_code, m.error_message, m.provider, m.model, m.usage.as_ref().map(json).transpose()?],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(format!("message '{}'", m.id)));
        }
        Ok(())
    }

    fn list_messages(&self, conversation_id: &str) -> AppResult<Vec<StoredMessage>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {MSG_COLS} FROM ai_messages WHERE conversation_id=?1 ORDER BY seq ASC"))?;
        let mut rows = stmt.query(params![conversation_id])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            match message_from_row(r) {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!("skipping an unreadable message: {e}"),
            }
        }
        Ok(out)
    }

    fn delete_message(&self, id: &str) -> AppResult<()> {
        self.conn()?.execute("DELETE FROM ai_messages WHERE id=?1", params![id])?;
        Ok(())
    }

    fn purge_conversations_before(&self, cutoff: &str) -> AppResult<usize> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM ai_messages WHERE conversation_id IN (SELECT id FROM ai_conversations WHERE updated_at < ?1)", params![cutoff])?;
        Ok(conn.execute("DELETE FROM ai_conversations WHERE updated_at < ?1", params![cutoff])?)
    }

    fn put_turn(&self, message_id: &str, conversation_id: &str, json: &str) -> AppResult<()> {
        self.conn()?.execute("INSERT INTO ai_turns (message_id, conversation_id, turn_json) VALUES (?1, ?2, ?3) ON CONFLICT(message_id) DO UPDATE SET turn_json=excluded.turn_json", params![message_id, conversation_id, json])?;
        Ok(())
    }

    fn list_turns(&self, conversation_id: &str) -> AppResult<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT message_id, turn_json FROM ai_turns WHERE conversation_id=?1")?;
        let rows = stmt.query_map(params![conversation_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn close_streaming_messages(&self) -> AppResult<usize> {
        let conn = self.conn()?;
        Ok(conn.execute("UPDATE ai_messages SET status='interrupted' WHERE status='streaming'", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ai::{MessageStatus, Role};

    fn conv(id: &str, updated: &str) -> Conversation {
        Conversation { id: id.into(), title: "t".into(), provider: Some("openai".into()), model: Some("m".into()), created_at: updated.into(), updated_at: updated.into() }
    }

    fn msg(conv: &str, id: &str, role: Role, text: &str, status: MessageStatus) -> StoredMessage {
        StoredMessage { id: id.into(), conversation_id: conv.into(), seq: 0, role, content: text.into(), status, error_code: None, error_message: None, provider: None, model: None, usage: None, created_at: "2026-01-01T00:00:00Z".into() }
    }

    #[test]
    fn conversations_and_messages_round_trip_in_order_and_delete_together() {
        let s = SqliteAiStore::in_memory().unwrap();
        s.create_conversation(&conv("c1", "2026-01-02T00:00:00Z")).unwrap();
        s.create_conversation(&conv("c2", "2026-01-03T00:00:00Z")).unwrap();
        let a = s.append_message(&msg("c1", "m1", Role::User, "hello", MessageStatus::Complete)).unwrap();
        let b = s.append_message(&msg("c1", "m2", Role::Assistant, "hi", MessageStatus::Streaming)).unwrap();
        assert_eq!((a.seq, b.seq), (1, 2));
        let mut done = b.clone();
        done.content = "hi there".into();
        done.status = MessageStatus::Complete;
        done.usage = Some(Usage { input_tokens: Some(3), output_tokens: Some(4), total_tokens: Some(7), cost_usd: Some(0.001) });
        s.update_message(&done).unwrap();
        let list = s.list_messages("c1").unwrap();
        assert_eq!(list.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["hello", "hi there"]);
        assert_eq!(list[1].usage.as_ref().unwrap().cost_usd, Some(0.001));
        assert_eq!(s.list_conversations(10).unwrap()[0].id, "c2", "newest first");
        s.delete_conversation("c1").unwrap();
        assert!(s.list_messages("c1").unwrap().is_empty());
        assert!(matches!(s.get_conversation("c1"), Err(AppError::NotFound(_))));
        assert_eq!(s.delete_all_conversations().unwrap(), 1);
    }

    #[test]
    fn provider_settings_status_hint_and_active_provider_persist_without_any_secret_column() {
        let s = SqliteAiStore::in_memory().unwrap();
        assert_eq!(s.get_config("openai").unwrap(), None);
        let mut c = ProviderConfig::new("openai");
        c.default_model = Some("m".into());
        s.put_config(&c).unwrap();
        assert_eq!(s.get_config("openai").unwrap(), Some(c));
        s.put_key_hint("openai", Some("sk-\u{2022}\u{2022} (51 characters)")).unwrap();
        assert!(s.get_key_hint("openai").unwrap().unwrap().contains("51"));
        let mut st = ProviderStatus::default();
        st.state = ConnectionState::Verified;
        st.verified_models = vec!["m".into()];
        s.put_status("openai", &st).unwrap();
        assert_eq!(s.get_status("openai").unwrap().state, ConnectionState::Verified);
        assert_eq!(s.active_provider().unwrap(), None);
        s.set_active_provider(Some("openai")).unwrap();
        assert_eq!(s.active_provider().unwrap().as_deref(), Some("openai"));
        s.set_active_provider(None).unwrap();
        assert_eq!(s.active_provider().unwrap(), None);
        // The schema has nowhere to keep a key.
        let conn = s.conn().unwrap();
        for table in ["ai_providers", "ai_meta", "ai_conversations", "ai_messages"] {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let cols: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap().map(|c| c.unwrap().to_lowercase()).collect();
            assert!(!cols.iter().any(|c| c.contains("key") && c != "key_hint" && c != "key" || c.contains("secret") || c.contains("token_value") || c.contains("password")), "{table}: {cols:?}");
        }
    }

    #[test]
    fn retention_deletes_old_conversations_with_their_messages_and_restart_closes_streaming_messages() {
        let s = SqliteAiStore::in_memory().unwrap();
        s.create_conversation(&conv("old", "2026-01-01T00:00:00Z")).unwrap();
        s.create_conversation(&conv("new", "2026-06-01T00:00:00Z")).unwrap();
        s.append_message(&msg("old", "a", Role::User, "x", MessageStatus::Complete)).unwrap();
        s.append_message(&msg("new", "b", Role::Assistant, "partial", MessageStatus::Streaming)).unwrap();
        assert_eq!(s.purge_conversations_before("2026-03-01T00:00:00Z").unwrap(), 1);
        assert!(s.list_messages("old").unwrap().is_empty());
        assert_eq!(s.close_streaming_messages().unwrap(), 1);
        let m = &s.list_messages("new").unwrap()[0];
        assert_eq!((m.status, m.content.as_str()), (MessageStatus::Interrupted, "partial"), "partial text is kept");
    }

    #[test]
    fn an_unreadable_message_row_is_skipped_not_fatal() {
        let s = SqliteAiStore::in_memory().unwrap();
        s.create_conversation(&conv("c", "2026-01-01T00:00:00Z")).unwrap();
        s.append_message(&msg("c", "ok", Role::User, "fine", MessageStatus::Complete)).unwrap();
        s.conn().unwrap().execute("INSERT INTO ai_messages (id, conversation_id, seq, role, content, status, created_at) VALUES ('bad','c',2,'nonsense','x','???','t')", []).unwrap();
        assert_eq!(s.list_messages("c").unwrap().len(), 1);
    }
}
