//! Local persistence: SQLite (bundled, with FTS5 full-text search). Nothing leaves the computer.
use crate::models::*;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

pub struct Db { pub conn: Connection }

pub fn now() -> f64 { chrono::Utc::now().timestamp_millis() as f64 / 1000.0 }

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let v: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            self.conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS meetings(id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at REAL NOT NULL, duration REAL DEFAULT 0,
              audio_file TEXT DEFAULT '', original_file_name TEXT, source TEXT DEFAULT 'recording', status TEXT DEFAULT 'recorded',
              status_detail TEXT DEFAULT '', progress REAL DEFAULT 0, language TEXT DEFAULT 'auto', collection_id TEXT,
              is_favorite INTEGER DEFAULT 0, is_archived INTEGER DEFAULT 0, user_notes TEXT DEFAULT '', template_id TEXT DEFAULT 'general',
              stereo_sources INTEGER DEFAULT 0, transcribed_until REAL DEFAULT 0, title_user_set INTEGER DEFAULT 0, last_opened_at REAL);
            CREATE TABLE IF NOT EXISTS tags(meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, tag TEXT NOT NULL, PRIMARY KEY(meeting_id, tag));
            CREATE TABLE IF NOT EXISTS segments(id INTEGER PRIMARY KEY AUTOINCREMENT, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
              idx INTEGER NOT NULL, start_time REAL NOT NULL, end_time REAL NOT NULL, text TEXT NOT NULL, original_text TEXT NOT NULL, speaker TEXT);
            CREATE INDEX IF NOT EXISTS segments_meeting ON segments(meeting_id, idx);
            CREATE TABLE IF NOT EXISTS speakers(meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, key TEXT NOT NULL, name TEXT NOT NULL, PRIMARY KEY(meeting_id, key));
            CREATE TABLE IF NOT EXISTS highlights(id INTEGER PRIMARY KEY AUTOINCREMENT, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, time REAL NOT NULL, label TEXT DEFAULT '');
            CREATE TABLE IF NOT EXISTS summaries(id INTEGER PRIMARY KEY AUTOINCREMENT, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
              template_id TEXT, template_name TEXT, markdown TEXT NOT NULL, structured_json TEXT, model_name TEXT, created_at REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS chunk_notes(meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, level INTEGER NOT NULL,
              chunk_index INTEGER NOT NULL, transcript_hash TEXT NOT NULL, json TEXT NOT NULL, PRIMARY KEY(meeting_id, level, chunk_index));
            CREATE TABLE IF NOT EXISTS action_items(id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, task TEXT NOT NULL,
              owner TEXT DEFAULT '', due TEXT DEFAULT '', priority TEXT DEFAULT '', done INTEGER DEFAULT 0, notes TEXT DEFAULT '', source_time REAL,
              created_at REAL NOT NULL, user_modified INTEGER DEFAULT 0);
            CREATE TABLE IF NOT EXISTS chat_messages(id INTEGER PRIMARY KEY AUTOINCREMENT, meeting_id TEXT, role TEXT NOT NULL, content TEXT NOT NULL, created_at REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS collections(id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS templates(id TEXT PRIMARY KEY, name TEXT NOT NULL, instructions TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS mindmaps(meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE, json TEXT NOT NULL, created_at REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS tldrs(meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE, markdown TEXT NOT NULL, model_name TEXT, created_at REAL NOT NULL);
            CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(content, meeting_id UNINDEXED, kind UNINDEXED, ref UNINDEXED,
              tokenize='porter unicode61 remove_diacritics 2');
            PRAGMA user_version = 2;
            "#)?;
        }
        Ok(())
    }

    // ---------- meetings ----------
    fn meeting_row(r: &Row) -> rusqlite::Result<Meeting> {
        Ok(Meeting {
            id: r.get("id")?, title: r.get("title")?, created_at: r.get("created_at")?, duration: r.get("duration")?,
            audio_file: r.get("audio_file")?, original_file_name: r.get("original_file_name")?, source: r.get("source")?,
            status: r.get("status")?, status_detail: r.get("status_detail")?, progress: r.get("progress")?, language: r.get("language")?,
            collection_id: r.get("collection_id")?, is_favorite: r.get::<_, i64>("is_favorite")? != 0, is_archived: r.get::<_, i64>("is_archived")? != 0,
            user_notes: r.get("user_notes")?, template_id: r.get("template_id")?, stereo_sources: r.get::<_, i64>("stereo_sources")? != 0,
            transcribed_until: r.get("transcribed_until")?, title_user_set: r.get::<_, i64>("title_user_set")? != 0, tags: vec![],
        })
    }

    fn tags_for(&self, id: &str) -> Vec<String> {
        let mut st = self.conn.prepare_cached("SELECT tag FROM tags WHERE meeting_id=? ORDER BY tag").unwrap();
        st.query_map([id], |r| r.get(0)).map(|it| it.flatten().collect()).unwrap_or_default()
    }

    pub fn meetings(&self) -> Vec<Meeting> {
        let mut st = self.conn.prepare("SELECT * FROM meetings ORDER BY created_at DESC").unwrap();
        let mut list: Vec<Meeting> = st.query_map([], Self::meeting_row).map(|it| it.flatten().collect()).unwrap_or_default();
        for m in list.iter_mut() { m.tags = self.tags_for(&m.id); }
        list
    }

    pub fn meeting(&self, id: &str) -> Option<Meeting> {
        let mut m = self.conn.query_row("SELECT * FROM meetings WHERE id=?", [id], Self::meeting_row).optional().ok()??;
        m.tags = self.tags_for(id);
        Some(m)
    }

    pub fn save_meeting(&self, m: &Meeting) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meetings(id,title,created_at,duration,audio_file,original_file_name,source,status,status_detail,progress,language,
               collection_id,is_favorite,is_archived,user_notes,template_id,stereo_sources,transcribed_until,title_user_set)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
             ON CONFLICT(id) DO UPDATE SET title=?2,created_at=?3,duration=?4,audio_file=?5,original_file_name=?6,source=?7,status=?8,
               status_detail=?9,progress=?10,language=?11,collection_id=?12,is_favorite=?13,is_archived=?14,user_notes=?15,template_id=?16,
               stereo_sources=?17,transcribed_until=?18,title_user_set=?19",
            params![m.id, m.title, m.created_at, m.duration, m.audio_file, m.original_file_name, m.source, m.status, m.status_detail,
                    m.progress, m.language, m.collection_id, m.is_favorite, m.is_archived, m.user_notes, m.template_id,
                    m.stereo_sources, m.transcribed_until, m.title_user_set])?;
        self.conn.execute("DELETE FROM tags WHERE meeting_id=?", [&m.id])?;
        for t in &m.tags {
            let t = t.trim();
            if !t.is_empty() { self.conn.execute("INSERT OR IGNORE INTO tags VALUES(?,?)", params![m.id, t])?; }
        }
        self.reindex(&m.id, "title", &[(String::new(), format!("{} {}", m.title, m.tags.join(" ")))]);
        self.reindex(&m.id, "notes", &[(String::new(), m.user_notes.clone())]);
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: &str, detail: &str, progress: f64) {
        let _ = self.conn.execute("UPDATE meetings SET status=?, status_detail=?, progress=? WHERE id=?", params![status, detail, progress, id]);
    }
    pub fn set_transcribed_until(&self, id: &str, t: f64) {
        let _ = self.conn.execute("UPDATE meetings SET transcribed_until=? WHERE id=?", params![t, id]);
    }
    pub fn delete_meeting(&self, id: &str) {
        let _ = self.conn.execute("DELETE FROM search_index WHERE meeting_id=?", [id]);
        let _ = self.conn.execute("DELETE FROM chat_messages WHERE meeting_id=?", [id]);
        let _ = self.conn.execute("DELETE FROM meetings WHERE id=?", [id]);
        let _ = std::fs::remove_dir_all(crate::paths::recordings().join(id));
    }
    pub fn mark_interrupted(&self) {
        let _ = self.conn.execute("UPDATE meetings SET status='interrupted', status_detail='Processing stopped when the app closed. Click Resume to continue.'
            WHERE status IN ('queued','preparing','transcribing','summarizing')", []);
    }

    // ---------- segments & speakers ----------
    pub fn segments(&self, id: &str) -> Vec<Segment> {
        let mut st = self.conn.prepare_cached("SELECT * FROM segments WHERE meeting_id=? ORDER BY idx").unwrap();
        st.query_map([id], |r| Ok(Segment { id: r.get("id")?, index: r.get("idx")?, start: r.get("start_time")?, end: r.get("end_time")?,
            text: r.get("text")?, original_text: r.get("original_text")?, speaker: r.get("speaker")? }))
            .map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn clear_segments_from(&self, id: &str, t: f64) {
        let _ = self.conn.execute("DELETE FROM segments WHERE meeting_id=? AND start_time>=?", params![id, t]);
    }
    pub fn append_segments(&self, id: &str, segs: &[(f64, f64, String, Option<String>)]) -> Result<()> {
        let start: i64 = self.conn.query_row("SELECT COALESCE(MAX(idx),-1)+1 FROM segments WHERE meeting_id=?", [id], |r| r.get(0))?;
        let tx = self.conn.unchecked_transaction()?;
        for (i, (s, e, t, sp)) in segs.iter().enumerate() {
            tx.execute("INSERT INTO segments(meeting_id,idx,start_time,end_time,text,original_text,speaker) VALUES(?,?,?,?,?,?,?)",
                       params![id, start + i as i64, s, e, t, t, sp])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn update_segment(&self, seg_id: i64, text: Option<&str>, speaker: Option<Option<&str>>) {
        if let Some(t) = text { let _ = self.conn.execute("UPDATE segments SET text=? WHERE id=?", params![t, seg_id]); }
        if let Some(sp) = speaker { let _ = self.conn.execute("UPDATE segments SET speaker=? WHERE id=?", params![sp, seg_id]); }
    }
    pub fn revert_segment(&self, seg_id: i64) { let _ = self.conn.execute("UPDATE segments SET text=original_text WHERE id=?", [seg_id]); }
    pub fn speakers(&self, id: &str) -> std::collections::HashMap<String, String> {
        let mut st = self.conn.prepare_cached("SELECT key,name FROM speakers WHERE meeting_id=?").unwrap();
        st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?))).map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn set_speaker(&self, id: &str, key: &str, name: &str) {
        let _ = self.conn.execute("INSERT INTO speakers VALUES(?,?,?) ON CONFLICT(meeting_id,key) DO UPDATE SET name=excluded.name", params![id, key, name]);
    }

    // ---------- highlights ----------
    pub fn highlights(&self, id: &str) -> Vec<Highlight> {
        let mut st = self.conn.prepare_cached("SELECT id,time,label FROM highlights WHERE meeting_id=? ORDER BY time").unwrap();
        st.query_map([id], |r| Ok(Highlight { id: r.get(0)?, time: r.get(1)?, label: r.get(2)? })).map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn add_highlight(&self, id: &str, time: f64, label: &str) {
        let _ = self.conn.execute("INSERT INTO highlights(meeting_id,time,label) VALUES(?,?,?)", params![id, time, label]);
    }
    pub fn delete_highlight(&self, hid: i64) { let _ = self.conn.execute("DELETE FROM highlights WHERE id=?", [hid]); }

    // ---------- summaries / tldr / mindmap / chunk cache ----------
    pub fn summaries(&self, id: &str) -> Vec<Summary> {
        let mut st = self.conn.prepare_cached("SELECT * FROM summaries WHERE meeting_id=? ORDER BY created_at DESC").unwrap();
        st.query_map([id], |r| Ok(Summary { id: r.get("id")?, template_id: r.get("template_id")?, template_name: r.get("template_name")?,
            markdown: r.get("markdown")?, structured_json: r.get("structured_json")?, model_name: r.get("model_name")?, created_at: r.get("created_at")? }))
            .map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn add_summary(&self, id: &str, s: &Summary) {
        let _ = self.conn.execute("INSERT INTO summaries(meeting_id,template_id,template_name,markdown,structured_json,model_name,created_at) VALUES(?,?,?,?,?,?,?)",
            params![id, s.template_id, s.template_name, s.markdown, s.structured_json, s.model_name, now()]);
        self.reindex(id, "summary", &[(String::new(), s.markdown.clone())]);
    }
    pub fn update_summary(&self, id: &str, sid: i64, md: &str) {
        let _ = self.conn.execute("UPDATE summaries SET markdown=? WHERE id=?", params![md, sid]);
        self.reindex(id, "summary", &[(String::new(), md.to_string())]);
    }
    pub fn tldr(&self, id: &str) -> Option<(String, String, f64)> {
        self.conn.query_row("SELECT markdown, model_name, created_at FROM tldrs WHERE meeting_id=?", [id],
            |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?))).optional().ok().flatten()
    }
    pub fn save_tldr(&self, id: &str, md: &str, model: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO tldrs VALUES(?,?,?,?)", params![id, md, model, now()]);
    }
    pub fn mindmap(&self, id: &str) -> Option<String> {
        self.conn.query_row("SELECT json FROM mindmaps WHERE meeting_id=?", [id], |r| r.get(0)).optional().ok().flatten()
    }
    pub fn save_mindmap(&self, id: &str, json: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO mindmaps VALUES(?,?,?)", params![id, json, now()]);
    }
    pub fn chunk_note(&self, id: &str, level: i64, idx: i64, hash: &str) -> Option<String> {
        self.conn.query_row("SELECT json FROM chunk_notes WHERE meeting_id=? AND level=? AND chunk_index=? AND transcript_hash=?",
            params![id, level, idx, hash], |r| r.get(0)).optional().ok().flatten()
    }
    pub fn save_chunk_note(&self, id: &str, level: i64, idx: i64, hash: &str, json: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO chunk_notes VALUES(?,?,?,?,?)", params![id, level, idx, hash, json]);
    }

    // ---------- action items ----------
    pub fn action_items(&self, id: Option<&str>) -> Vec<ActionItem> {
        let map = |r: &Row| Ok(ActionItem { id: r.get("id")?, meeting_id: r.get("meeting_id")?, task: r.get("task")?, owner: r.get("owner")?,
            due: r.get("due")?, priority: r.get("priority")?, done: r.get::<_, i64>("done")? != 0, notes: r.get("notes")?, source_time: r.get("source_time")? });
        match id {
            Some(id) => self.conn.prepare("SELECT * FROM action_items WHERE meeting_id=? ORDER BY done, created_at").unwrap()
                .query_map([id], map).map(|it| it.flatten().collect()).unwrap_or_default(),
            None => self.conn.prepare("SELECT a.* FROM action_items a JOIN meetings m ON m.id=a.meeting_id WHERE m.is_archived=0 ORDER BY a.done, m.created_at DESC").unwrap()
                .query_map([], map).map(|it| it.flatten().collect()).unwrap_or_default(),
        }
    }
    pub fn save_action_item(&self, a: &ActionItem, user_modified: bool) {
        let _ = self.conn.execute("INSERT INTO action_items(id,meeting_id,task,owner,due,priority,done,notes,source_time,created_at,user_modified)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET task=?3,owner=?4,due=?5,priority=?6,done=?7,notes=?8,source_time=?9,user_modified=?11",
            params![a.id, a.meeting_id, a.task, a.owner, a.due, a.priority, a.done, a.notes, a.source_time, now(), user_modified]);
        self.reindex_actions(&a.meeting_id);
    }
    pub fn delete_action_item(&self, aid: &str, mid: &str) {
        let _ = self.conn.execute("DELETE FROM action_items WHERE id=?", [aid]);
        self.reindex_actions(mid);
    }
    /// Replaces AI-extracted items but keeps items the user completed or edited.
    pub fn replace_generated_actions(&self, mid: &str, items: &[ActionItem]) {
        let _ = self.conn.execute("DELETE FROM action_items WHERE meeting_id=? AND user_modified=0 AND done=0", [mid]);
        let existing: Vec<String> = self.action_items(Some(mid)).iter().map(|a| a.task.to_lowercase()).collect();
        for it in items { if !existing.contains(&it.task.to_lowercase()) { self.save_action_item(it, false); } }
    }
    fn reindex_actions(&self, mid: &str) {
        let rows: Vec<(String, String)> = self.action_items(Some(mid)).iter()
            .map(|a| (a.id.clone(), [a.task.as_str(), a.owner.as_str(), a.due.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" — "))).collect();
        self.reindex(mid, "action", &rows);
    }

    // ---------- chat, collections, templates ----------
    pub fn chat(&self, mid: Option<&str>) -> Vec<ChatMessage> {
        let map = |r: &Row| Ok(ChatMessage { id: r.get("id")?, meeting_id: r.get("meeting_id")?, role: r.get("role")?, content: r.get("content")?, created_at: r.get("created_at")? });
        match mid {
            Some(m) => self.conn.prepare("SELECT * FROM chat_messages WHERE meeting_id=? ORDER BY id").unwrap().query_map([m], map).map(|it| it.flatten().collect()).unwrap_or_default(),
            None => self.conn.prepare("SELECT * FROM chat_messages WHERE meeting_id IS NULL ORDER BY id").unwrap().query_map([], map).map(|it| it.flatten().collect()).unwrap_or_default(),
        }
    }
    pub fn add_chat(&self, mid: Option<&str>, role: &str, content: &str) -> i64 {
        let _ = self.conn.execute("INSERT INTO chat_messages(meeting_id,role,content,created_at) VALUES(?,?,?,?)", params![mid, role, content, now()]);
        self.conn.last_insert_rowid()
    }
    pub fn clear_chat(&self, mid: Option<&str>) {
        let _ = match mid { Some(m) => self.conn.execute("DELETE FROM chat_messages WHERE meeting_id=?", [m]),
                            None => self.conn.execute("DELETE FROM chat_messages WHERE meeting_id IS NULL", []) };
    }
    pub fn collections(&self) -> Vec<Collection> {
        self.conn.prepare("SELECT id,name FROM collections ORDER BY name COLLATE NOCASE").unwrap()
            .query_map([], |r| Ok(Collection { id: r.get(0)?, name: r.get(1)? })).map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn save_collection(&self, c: &Collection) { let _ = self.conn.execute("INSERT OR REPLACE INTO collections VALUES(?,?)", params![c.id, c.name]); }
    pub fn delete_collection(&self, id: &str) {
        let _ = self.conn.execute("UPDATE meetings SET collection_id=NULL WHERE collection_id=?", [id]);
        let _ = self.conn.execute("DELETE FROM collections WHERE id=?", [id]);
    }
    pub fn custom_templates(&self) -> Vec<(String, String, String)> {
        self.conn.prepare("SELECT id,name,instructions FROM templates ORDER BY name").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|it| it.flatten().collect()).unwrap_or_default()
    }
    pub fn save_template(&self, id: &str, name: &str, ins: &str) { let _ = self.conn.execute("INSERT OR REPLACE INTO templates VALUES(?,?,?)", params![id, name, ins]); }
    pub fn delete_template(&self, id: &str) { let _ = self.conn.execute("DELETE FROM templates WHERE id=?", [id]); }

    // ---------- full-text search ----------
    fn reindex(&self, mid: &str, kind: &str, rows: &[(String, String)]) {
        let _ = self.conn.execute("DELETE FROM search_index WHERE meeting_id=? AND kind=?", params![mid, kind]);
        for (r, c) in rows {
            if !c.trim().is_empty() {
                let _ = self.conn.execute("INSERT INTO search_index(content,meeting_id,kind,ref) VALUES(?,?,?,?)", params![c, mid, kind, r]);
            }
        }
    }
    pub fn reindex_segments(&self, mid: &str) {
        let names = self.speakers(mid);
        let rows: Vec<(String, String)> = self.segments(mid).iter().map(|s| {
            let who = s.speaker.as_ref().map(|k| names.get(k).cloned().unwrap_or_else(|| crate::ai::speaker_default_name(k)));
            (format!("{}", s.start), match who { Some(w) => format!("{w}: {}", s.text), None => s.text.clone() })
        }).collect();
        self.reindex(mid, "segment", &rows);
    }

    /// Free text → safe FTS5 query (quoted prefix terms).
    pub fn fts_query(text: &str, any: bool) -> Option<String> {
        let words: Vec<String> = text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 2)
            .take(24).map(|w| format!("\"{w}\"*")).collect();
        if words.is_empty() { None } else { Some(words.join(if any { " OR " } else { " " })) }
    }

    pub fn search(&self, text: &str, kinds: Option<&[&str]>, mid: Option<&str>, limit: usize, any: bool) -> Vec<SearchHit> {
        let Some(q) = Self::fts_query(text, any) else { return vec![] };
        let mut sql = String::from("SELECT meeting_id, kind, ref, snippet(search_index,0,'«','»','…',14) FROM search_index WHERE search_index MATCH ?1");
        if let Some(m) = mid { sql += &format!(" AND meeting_id='{}'", m.replace('\'', "")); }
        if let Some(k) = kinds { sql += &format!(" AND kind IN ({})", k.iter().map(|x| format!("'{x}'")).collect::<Vec<_>>().join(",")); }
        sql += &format!(" ORDER BY rank LIMIT {limit}");
        let Ok(mut st) = self.conn.prepare(&sql) else { return vec![] };
        st.query_map([q], |r| Ok(SearchHit { meeting_id: r.get(0)?, kind: r.get(1)?, r#ref: r.get(2)?, snippet: r.get(3)? }))
            .map(|it| it.flatten().collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_fts() {
        let dir = std::env::temp_dir().join(format!("omn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMN_DATA_DIR", &dir);
        let db = Db::open(&dir.join("t.sqlite")).unwrap();
        let m = Meeting { id: "m1".into(), title: "Budget review".into(), status: "recorded".into(), source: "recording".into(),
                          language: "auto".into(), template_id: "general".into(), tags: vec!["finance".into()], ..Default::default() };
        db.save_meeting(&m).unwrap();
        db.append_segments("m1", &[(12.0, 15.0, "We approved the retreat budget.".into(), None)]).unwrap();
        db.reindex_segments("m1");
        assert_eq!(db.meetings()[0].tags, vec!["finance"]);
        let hits = db.search("approved budgets", None, None, 50, false);
        assert!(hits.iter().any(|h| h.kind == "segment" && h.r#ref == "12"), "{hits:?}");
        assert_eq!(Db::fts_query("budget \"OR\" (drop)", false).unwrap(), "\"budget\"* \"or\"* \"drop\"*");
        db.delete_meeting("m1");
        assert!(db.meetings().is_empty());
        assert!(db.search("approved", None, None, 10, false).is_empty());
    }
}
