use crate::{
    model::*,
    rpc::{item_text, text, RpcHandle},
};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::mpsc;

const LIMIT: usize = 100;
// A replacement search waits for its cancelled predecessor to leave the index.
// This keeps partially indexed histories from racing with a newer refresh.
static INDEX_WRITER: Mutex<()> = Mutex::new(());
fn database() -> Result<Connection> {
    let path = state_dir().join("message-search.sqlite");
    let db = Connection::open(&path)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=3000;
        CREATE TABLE IF NOT EXISTS threads(id TEXT PRIMARY KEY,title TEXT,cwd TEXT,archived INTEGER,stamp TEXT,updated INTEGER,visible INTEGER);
        CREATE TABLE IF NOT EXISTS docs(thread TEXT,mid TEXT,turn_id TEXT,turn_order INTEGER,item_order INTEGER,role TEXT,body TEXT,folded TEXT,UNIQUE(thread,mid));
        CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(folded,content='docs',content_rowid='rowid',tokenize='trigram case_sensitive 1');
        CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN INSERT INTO docs_fts(rowid,folded) VALUES(new.rowid,new.folded); END;
        CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN INSERT INTO docs_fts(docs_fts,rowid,folded) VALUES('delete',old.rowid,old.folded); END;
        CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
            INSERT INTO docs_fts(docs_fts,rowid,folded) VALUES('delete',old.rowid,old.folded);
            INSERT INTO docs_fts(rowid,folded) VALUES(new.rowid,new.folded); END;")?;
    Ok(db)
}
fn excerpt(body: &str, query: &str) -> String {
    let folded = body.to_lowercase();
    let offset = folded.find(&query.to_lowercase()).unwrap_or(0);
    let chars = folded[..offset].chars().count();
    let start = chars.saturating_sub(45);
    let part: String = body.chars().skip(start).take(180).collect();
    format!(
        "{}{}",
        if start > 0 { "…" } else { "" },
        part.replace(['\n', '\r', '\t'], " ")
    )
}
fn hits(db: &Connection, query: &str) -> Result<(Vec<SearchHit>, bool)> {
    let query = query.to_lowercase();
    let use_fts = query.chars().count() >= 3;
    let filter = if use_fts {
        "JOIN docs_fts f ON f.rowid=d.rowid WHERE f.folded MATCH ?1"
    } else {
        "WHERE instr(d.folded,?1)>0"
    };
    let sql = format!("SELECT d.thread,d.mid,t.title,t.cwd,d.role,d.body,t.archived FROM docs d JOIN threads t ON t.id=d.thread {filter} AND t.visible=1 ORDER BY t.updated DESC,d.thread,d.turn_order,d.item_order LIMIT {}", LIMIT+1);
    let term = if use_fts {
        format!("\"{}\"", query.replace('"', "\"\""))
    } else {
        query.clone()
    };
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map([term], |r| {
        Ok(SearchHit {
            thread: r.get(0)?,
            message: r.get(1)?,
            title: r.get(2)?,
            cwd: r.get(3)?,
            role: r.get(4)?,
            excerpt: r.get(5)?,
            archived: r.get(6)?,
        })
    })?;
    let mut out = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    let truncated = out.len() > LIMIT;
    out.truncate(LIMIT);
    for hit in &mut out {
        hit.excerpt = excerpt(&hit.excerpt, &query);
    }
    Ok((out, truncated))
}
fn insert(
    db: &Connection,
    thread: &str,
    turn: &str,
    turn_order: i64,
    item_order: usize,
    m: &Message,
) -> Result<()> {
    if m.text.is_empty() {
        return Ok(());
    }
    db.execute("INSERT INTO docs(thread,mid,turn_id,turn_order,item_order,role,body,folded) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(thread,mid) DO UPDATE SET turn_id=excluded.turn_id,turn_order=excluded.turn_order,item_order=excluded.item_order,role=excluded.role,body=excluded.body,folded=excluded.folded",
        params![thread,m.id,turn,turn_order,item_order,m.role,m.text,m.text.to_lowercase()])?;
    Ok(())
}
fn turns(db: &mut Connection, thread: &str, data: &[Value], order: &mut i64) -> Result<()> {
    let tx = db.transaction()?;
    for turn in data {
        if let Some(items) = turn["items"].as_array() {
            for (i, item) in items.iter().enumerate() {
                let role = match text(item, "type").as_str() {
                    "userMessage" => "You".into(),
                    "agentMessage" => "Codex".into(),
                    other => other.into(),
                };
                let mut mid = text(item, "id");
                if mid.is_empty() {
                    mid = format!("{}-{i}", text(turn, "id"));
                }
                insert(
                    &tx,
                    thread,
                    &text(turn, "id"),
                    *order,
                    i,
                    &Message {
                        id: mid,
                        role,
                        text: item_text(item),
                        status: text(item, "status"),
                    },
                )?;
            }
        }
        *order += 1;
    }
    tx.commit()?;
    Ok(())
}
fn publish(
    db: &Connection,
    state: &mut SearchState,
    events: &mpsc::UnboundedSender<Value>,
    generation: u64,
) -> Result<()> {
    (state.hits, state.truncated) = hits(db, &state.query)?;
    events.send(
        json!({"method":"sessiondesk/searchProgress","_generation":generation,"params":state}),
    )?;
    Ok(())
}
pub fn start(
    rpc: RpcHandle,
    events: mpsc::UnboundedSender<Value>,
    generation: u64,
    seeds: BTreeMap<String, Conversation>,
    job: String,
    query: String,
    cancel: Arc<AtomicBool>,
) {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let mut state = SearchState {
            job,
            query,
            running: true,
            ..Default::default()
        };
        let result = (|| -> Result<()> {
            let _writer = INDEX_WRITER
                .lock()
                .map_err(|_| anyhow!("Search index unavailable"))?;
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let mut db = database()?;
            let call = |method: &str, args: Value| -> Result<Value> {
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Search cancelled");
                let result = runtime.block_on(rpc.call(method, args));
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Search cancelled");
                result
            };
            // Cached results are immediately usable while the global index refreshes.
            publish(&db, &mut state, &events, generation)?;
            let mut known: BTreeMap<String, (Value, bool)> = BTreeMap::new();
            for archived in [false, true] {
                let mut cursor = Value::Null;
                let mut seen = BTreeSet::new();
                loop {
                    let response = call(
                        "thread/list",
                        json!({"limit":100,"cursor":cursor,"archived":archived,"sourceKinds":["cli","vscode","appServer"],"sortKey":"updated_at"}),
                    )?;
                    if let Some(data) = response["data"].as_array() {
                        for t in data {
                            known.insert(text(t, "id"), (t.clone(), archived));
                        }
                    }
                    cursor = response["nextCursor"].clone();
                    if cursor.is_null() {
                        break;
                    }
                    anyhow::ensure!(
                        seen.insert(cursor.to_string()),
                        "Backend repeated a thread-list cursor"
                    );
                }
            }
            for c in seeds.values().filter(|c| c.id.starts_with("local-")) {
                known.insert(
                    c.id.clone(),
                    (
                        json!({"id":c.id,"name":c.title,"cwd":c.cwd,"updatedAt":c.updated}),
                        c.archived,
                    ),
                );
            }
            state.total = known.len();
            let tx = db.transaction()?;
            tx.execute("UPDATE threads SET visible=0", [])?;
            for (key, (t, archived)) in &known {
                let name = t["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| t["preview"].as_str())
                    .unwrap_or("Untitled conversation");
                tx.execute("INSERT INTO threads(id,title,cwd,archived,stamp,updated,visible) VALUES(?1,?2,?3,?4,'',?5,1) ON CONFLICT(id) DO UPDATE SET title=excluded.title,cwd=excluded.cwd,archived=excluded.archived,updated=excluded.updated,visible=1",params![key,name,text(t,"cwd"),archived,t["updatedAt"].as_i64().unwrap_or(0)])?;
            }
            tx.commit()?;
            for (key, (t, _)) in known {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let stamp = if let Some(c) = seeds.get(&key).filter(|c| c.id.starts_with("local-"))
                {
                    let mut hash = DefaultHasher::new();
                    serde_json::to_vec(&c.messages)?.hash(&mut hash);
                    format!("{}:{:016x}", c.updated, hash.finish())
                } else {
                    t["updatedAt"].to_string()
                };
                let previous: String =
                    db.query_row("SELECT stamp FROM threads WHERE id=?1", [&key], |r| {
                        r.get(0)
                    })?;
                let force = seeds.get(&key).is_some_and(Conversation::busy);
                if previous != stamp || force {
                    let indexed = (|| -> Result<()> {
                        db.execute("UPDATE threads SET stamp='' WHERE id=?1", [&key])?;
                        db.execute("DELETE FROM docs WHERE thread=?1", [&key])?;
                        if key.starts_with("local-") {
                            if let Some(c) = seeds.get(&key) {
                                let tx = db.transaction()?;
                                for (i, m) in c.messages.iter().enumerate() {
                                    insert(&tx, &key, "", 0, i, m)?;
                                }
                                tx.commit()?;
                            }
                        } else {
                            let meta =
                                call("thread/read", json!({"threadId":key,"includeTurns":false}))?;
                            let mut order = 0;
                            if meta["thread"]["historyMode"] == "paginated"
                                || t["historyMode"] == "paginated"
                            {
                                let mut cursor = Value::Null;
                                let mut seen = BTreeSet::new();
                                loop {
                                    let page = call(
                                        "thread/turns/list",
                                        json!({"threadId":key,"limit":100,"cursor":cursor,"itemsView":"full","sortDirection":"desc"}),
                                    )?;
                                    if let Some(data) = page["data"].as_array() {
                                        turns(&mut db, &key, data, &mut order)?;
                                    }
                                    cursor = page["nextCursor"].clone();
                                    if cursor.is_null() {
                                        break;
                                    }
                                    anyhow::ensure!(
                                        seen.insert(cursor.to_string()),
                                        "Backend repeated a history cursor"
                                    );
                                }
                            } else {
                                let full = call(
                                    "thread/read",
                                    json!({"threadId":key,"includeTurns":true}),
                                )?;
                                let mut data = full["thread"]["turns"]
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default();
                                data.reverse();
                                turns(&mut db, &key, &data, &mut order)?;
                            }
                        }
                        db.execute(
                            "UPDATE threads SET stamp=?2 WHERE id=?1",
                            params![key, stamp],
                        )?;
                        Ok(())
                    })();
                    if let Err(e) = indexed {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        state.errors += 1;
                        state.error = format!("Some histories could not be read: {e:#}");
                    }
                }
                state.completed += 1;
                publish(&db, &mut state, &events, generation)?;
            }
            state.running = false;
            publish(&db, &mut state, &events, generation)?;
            Ok(())
        })();
        if let Err(e) = result {
            if !cancel.load(Ordering::Relaxed) {
                state.running = false;
                state.errors += 1;
                state.error = format!("Search failed: {e:#}");
                let _=events.send(json!({"method":"sessiondesk/searchProgress","_generation":generation,"params":state}));
            }
        }
    });
}
pub fn context(thread: &str, message: &str) -> Result<SearchContext> {
    let db = database()?;
    let (title,cwd,archived,order): (String,String,bool,i64)=db.query_row("SELECT t.title,t.cwd,t.archived,d.turn_order FROM docs d JOIN threads t ON t.id=d.thread WHERE d.thread=?1 AND d.mid=?2",params![thread,message],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).context("Search result changed; run the search again")?;
    let mut statement=db.prepare("SELECT mid,role,body FROM docs WHERE thread=?1 AND turn_order BETWEEN ?2 AND ?3 ORDER BY turn_order DESC,item_order")?;
    let messages = statement
        .query_map(params![thread, order - 1, order + 1], |r| {
            Ok(Message {
                id: r.get(0)?,
                role: r.get(1)?,
                text: r.get(2)?,
                status: String::new(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let found = messages
        .iter()
        .find(|m| m.id == message)
        .ok_or_else(|| anyhow!("Search result no longer available"))?;
    Ok(SearchContext {
        hit: SearchHit {
            thread: thread.into(),
            message: message.into(),
            title,
            cwd,
            archived,
            role: found.role.clone(),
            excerpt: String::new(),
        },
        messages,
    })
}
