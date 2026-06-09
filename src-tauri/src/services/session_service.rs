use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub session: Session,
    pub messages: Vec<ChatMessageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub session_id: String,
    pub session_title: String,
    pub matched_line: String,
    pub score: f64,
}

pub struct SessionService {
    db: Mutex<Connection>,
    sessions_json_path: PathBuf,
}

impl SessionService {
    pub fn new(hermes_home: &PathBuf) -> Result<Self, String> {
        let db_path = hermes_home.join("state.db");
        let db = Connection::open(&db_path)
            .map_err(|e| format!("打开数据库失败: {e}"))?;

        // Create base schema if Agent hasn't run yet.
        // The Agent (hermes_state.py) owns the full schema; these are minimal
        // stubs so the desktop app can function before the first agent start.
        db.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| format!("数据库初始化失败: {e}"))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                model TEXT,
                title TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                message_count INTEGER DEFAULT 0,
                agent_id TEXT
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content TEXT,
                timestamp REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);",
        )
        .map_err(|e| format!("数据库初始化失败: {e}"))?;

        // Migration: add agent_id column for older schemas
        let _ = db.execute("ALTER TABLE sessions ADD COLUMN agent_id TEXT", []);

        Ok(Self {
            db: Mutex::new(db),
            sessions_json_path: hermes_home.join("sessions.json"),
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db
            .prepare(
                "SELECT id, COALESCE(title,'') as title, COALESCE(model,'') as model,
                        datetime(started_at, 'unixepoch') as created_at,
                        COALESCE(datetime(ended_at, 'unixepoch'), datetime(started_at, 'unixepoch')) as updated_at,
                        COALESCE(message_count,0) as message_count
                 FROM sessions
                 WHERE started_at IS NOT NULL
                 ORDER BY started_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let sessions = stmt
            .query_map([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    model: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    message_count: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    pub fn get_session(&self, session_id: &str) -> Result<SessionDetail, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;

        let session = db
            .query_row(
                "SELECT id, COALESCE(title,'') as title, COALESCE(model,'') as model,
                        datetime(started_at, 'unixepoch') as created_at,
                        COALESCE(datetime(ended_at, 'unixepoch'), datetime(started_at, 'unixepoch')) as updated_at,
                        COALESCE(message_count,0) as message_count
                 FROM sessions WHERE id = ?1",
                params![session_id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        model: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        message_count: row.get(5)?,
                    })
                },
            )
            .map_err(|e| format!("会话不存在: {e}"))?;

        let mut stmt = db
            .prepare(
                "SELECT CAST(id AS TEXT) as id, role,
                        COALESCE(content,'') as content,
                        datetime(timestamp, 'unixepoch') as timestamp
                 FROM messages
                 WHERE session_id = ?1
                   AND role IN ('user', 'assistant', 'system')
                 ORDER BY timestamp ASC, id ASC",
            )
            .map_err(|e| e.to_string())?;

        let messages = stmt
            .query_map(params![session_id], |row| {
                Ok(ChatMessageRecord {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(SessionDetail { session, messages })
    }

    pub fn search_sessions(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;

        // Sanitize query for FTS5: wrap each word in quotes with * suffix
        let fts_query = query
            .split_whitespace()
            .map(|w| format!("\"{}\"*", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");

        let mut stmt = db
            .prepare(
                "SELECT m.session_id, COALESCE(s.title,'') as title, m.content, bm25(messages_fts, 1.0) as score
                 FROM messages_fts f
                 JOIN messages m ON f.rowid = m.rowid
                 JOIN sessions s ON m.session_id = s.id
                 WHERE messages_fts MATCH ?1
                   AND m.role IN ('user', 'assistant', 'system')
                 ORDER BY score
                 LIMIT 50",
            )
            .map_err(|e| e.to_string())?;

        let results = stmt
            .query_map(params![fts_query], |row| {
                Ok(SearchResult {
                    session_id: row.get(0)?,
                    session_title: row.get(1)?,
                    matched_line: row.get::<_, String>(2).unwrap_or_default(),
                    score: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    pub fn rename_session(&self, session_id: &str, title: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        db.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        db.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn upsert_session(
        &self,
        id: &str,
        title: &str,
        model: &str,
        agent_id: Option<&str>,
        _created_at: &str,
        _updated_at: &str,
    ) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let now_epoch = chrono::Utc::now().timestamp() as f64;
        db.execute(
            "INSERT INTO sessions (id, source, title, model, agent_id, started_at, ended_at, message_count)
             VALUES (?1, 'api_server', ?2, ?3, ?4, ?5, NULL, 0)
             ON CONFLICT(id) DO UPDATE SET
                title=excluded.title, model=excluded.model, agent_id=excluded.agent_id",
            params![id, title, model, agent_id, now_epoch],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn add_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<String, String> {
        let now_epoch = chrono::Utc::now().timestamp() as f64;
        let db = self.db.lock().map_err(|e| e.to_string())?;

        db.execute(
            "INSERT INTO messages (session_id, role, content, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, now_epoch],
        )
        .map_err(|e| e.to_string())?;

        let msg_id: i64 = db.last_insert_rowid();

        // Update session message count
        db.execute(
            "UPDATE sessions SET message_count = message_count + 1 WHERE id = ?1",
            params![session_id],
        )
        .map_err(|e| e.to_string())?;

        Ok(msg_id.to_string())
    }
}
