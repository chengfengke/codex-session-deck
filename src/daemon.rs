use crate::{
    activity::{role, Activity},
    layout::Node,
    model::*,
    rpc::{item_text, text, Rpc},
    store::Store,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot, watch},
};

struct Command {
    op: String,
    args: Value,
    reply: oneshot::Sender<Result<Value, String>>,
    delivered: oneshot::Receiver<()>,
}
struct Host {
    snap: Snapshot,
    store: Store,
    rpc: Option<Rpc>,
    events: mpsc::UnboundedSender<Value>,
    dirty: bool,
    keys: HashMap<String, PathBuf>,
    generation: u64,
    loads: HashMap<String, HistoryTask>,
    search_cancel: Option<Arc<AtomicBool>>,
    activity: Activity,
}
struct HistoryTask {
    token: String,
    task: tokio::task::JoinHandle<Result<LoadedHistory>>,
}
struct LoadedHistory {
    metadata: Option<Value>,
    history: Value,
    paginated: bool,
    older: bool,
    previous_turn: Option<String>,
}

pub async fn run() -> Result<()> {
    let store = Store::open()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state_dir().join("daemon.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let _ = std::fs::remove_file(socket_path());
    let listener = UnixListener::bind(socket_path())?;
    std::fs::set_permissions(socket_path(), std::fs::Permissions::from_mode(0o600))?;
    let mut data = store.load()?;
    for c in data.conversations.values_mut() {
        c.loaded = false;
        c.resumed = false;
        if c.busy() {
            c.status = "unknown".into();
            c.active_turn = None;
        }
        for q in &mut c.queued {
            q.state = "held".into();
        }
    }
    let (etx, mut erx) = mpsc::unbounded_channel();
    let (ctx, mut crx) = mpsc::channel::<Command>(128);
    let mut host = Host {
        snap: Snapshot {
            data,
            ..Default::default()
        },
        store,
        rpc: None,
        events: etx,
        dirty: true,
        keys: HashMap::new(),
        generation: 0,
        loads: HashMap::new(),
        search_cancel: None,
        activity: Activity::default(),
    };
    let (wtx, wrx) = watch::channel(host.snap.clone());
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tx = ctx.clone();
            let rx = wrx.clone();
            tokio::spawn(async move {
                let _ = serve(stream, tx, rx).await;
            });
        }
    });
    if let Err(e) = host.connect().await {
        host.snap.error = format!("{e:#}");
    }
    let mut paint = tokio::time::interval(Duration::from_millis(80));
    let mut save = tokio::time::interval(Duration::from_millis(600));
    let mut saved_seq = u64::MAX;
    loop {
        tokio::select! {
            Some(cmd)=crx.recv()=>{
                let shutdown=cmd.op=="shutdown";
                let changes = match cmd.op.as_str() {
                    "snapshot" => false,
                    "read" => host.snap.data.conversations.get(&text(&cmd.args, "thread")).is_some_and(|c| c.unread),
                    _ => true,
                };
                let result=host.command(&cmd.op,cmd.args).await.map_err(|e|format!("{e:#}"));
                let successful=result.is_ok();let _=cmd.reply.send(result);host.dirty |= changes;
                if shutdown&&successful{
                    if let Some(cancel)=host.search_cancel.take(){cancel.store(true,Ordering::Relaxed);}
                    if let Some(r)=host.rpc.as_mut(){r.stop().await;}
                    host.store.save(&host.snap.data)?;let _=tokio::time::timeout(Duration::from_secs(1),cmd.delivered).await;break;
                }
                if host.snap.connected{host.schedule().await;}
            },
            Some(event)=erx.recv()=>{
                if event["method"] == "sessiondesk/searchProgress" {
                    if event["_generation"].as_u64() == Some(host.generation) && event["params"]["job"] == host.snap.search.job {
                        if let Ok(state) = serde_json::from_value(event["params"].clone()) { host.snap.search = state; }
                    }
                } else if event["method"] == "sessiondesk/historyReady" {
                    if event["_generation"].as_u64() == Some(host.generation)
                        && host.loads.get(&text(&event["params"], "threadId")).is_some_and(|load| load.token == text(&event["params"], "loadId")) {
                        if let Err(e) = host.finish_load(&text(&event["params"], "threadId")).await { host.snap.error = format!("Cannot load history: {e:#}"); }
                    }
                } else { host.event(event); }
                host.dirty=true;if host.snap.connected{host.schedule().await;}
            },
            _=paint.tick()=>{if host.dirty{host.snap.seq+=1;wtx.send_replace(host.snap.clone());host.dirty=false;}},
            _=save.tick()=>{if saved_seq!=host.snap.seq{if let Err(e)=host.store.save(&host.snap.data){host.snap.error=format!("State save failed: {e}");host.dirty=true;}else{saved_seq=host.snap.seq;}}},
            _=tokio::signal::ctrl_c()=>{if let Some(cancel)=host.search_cancel.take(){cancel.store(true,Ordering::Relaxed);}host.store.save(&host.snap.data)?;break;}
        }
    }
    let _ = std::fs::remove_file(socket_path());
    Ok(())
}
async fn serve(
    stream: UnixStream,
    tx: mpsc::Sender<Command>,
    mut snapshots: watch::Receiver<Snapshot>,
) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut previous = snapshots.borrow_and_update().clone();
    let mut delta = false;
    let initial = json!({"type":"snapshot","snapshot":previous});
    write(&mut w, &initial).await?;
    loop {
        tokio::select! {
            line=lines.next_line()=>{
                let Some(line)=line?else{break};
                if line.len()>4*1024*1024{return Err(anyhow!("IPC frame exceeds 4 MiB"))}
                let v:Value=serde_json::from_str(&line)?;
                let op=text(&v,"op");
                if op == "subscribe" {
                    delta = v["args"]["delta"].as_bool().unwrap_or(false);
                    write(&mut w, &json!({"type":"response","id":v["id"],"result":{"delta":delta}})).await?;
                    continue;
                }
                let(rt,rr)=oneshot::channel();
                let(delivered,delivery)=oneshot::channel();
                tx.send(Command{op:op.clone(),args:v["args"].clone(),reply:rt,delivered:delivery}).await?;
                let result=rr.await?;
                let response=match result{Ok(r)=>json!({"type":"response","id":v["id"],"op":op,"result":r}),Err(e)=>json!({"type":"response","id":v["id"],"op":op,"error":e})};
                write(&mut w,&response).await?;
                let _ = delivered.send(());
            },
            changed=snapshots.changed()=>{
                if changed.is_err(){break}
                let current = snapshots.borrow_and_update().clone();
                let message = if delta {
                    json!({"type":"delta","delta":SnapshotDelta::between(&previous, &current)})
                } else {
                    json!({"type":"snapshot","snapshot":current})
                };
                write(&mut w,&message).await?;
                previous = current;
            }

        }
    }
    Ok(())
}
async fn write(w: &mut tokio::net::unix::OwnedWriteHalf, v: &Value) -> Result<()> {
    let mut b = serde_json::to_vec(v)?;
    b.push(b'\n');
    w.write_all(&b).await?;
    Ok(())
}

impl Host {
    fn rpc(&self) -> Result<&Rpc> {
        if !self.snap.connected {
            return Err(anyhow!(
                "Codex is offline; use /reconnect. Your drafts are saved."
            ));
        }
        self.rpc.as_ref().context("backend unavailable")
    }
    async fn connect(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.snap.connected
                || !self
                    .snap
                    .data
                    .conversations
                    .values()
                    .any(Conversation::busy),
            "Backend is connected and tasks are active; stop them before reconnecting."
        );
        self.generation += 1;
        self.activity.clear();
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.snap.search = SearchState::default();
        for (_, load) in self.loads.drain() {
            load.task.abort();
        }
        self.snap.loading.clear();
        for c in self.snap.data.conversations.values_mut() {
            c.loaded = false;
            c.resumed = false;
        }
        if let Some(old) = self.rpc.as_mut() {
            old.stop().await;
        }
        let rpc = Rpc::start(self.events.clone(), self.generation).await?;
        rpc.call("initialize",json!({"clientInfo":{"name":"codex_deck","title":"Codex Deck","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        rpc.notify("initialized", json!({}))?;
        self.snap.account = match rpc.call("account/read", json!({})).await {
            Ok(v) => v["account"].to_string(),
            Err(e) => e.to_string(),
        };
        self.snap.models = match rpc.call("model/list", json!({"limit":100})).await {
            Ok(v) => v["data"].as_array().cloned().unwrap_or_default(),
            Err(_) => vec![],
        };
        self.rpc = Some(rpc);
        self.snap.connected = true;
        self.snap.error.clear();
        self.snap.pending.clear();
        self.refresh(false).await?;
        self.dirty = true;
        Ok(())
    }
    async fn refresh(&mut self, archived: bool) -> Result<()> {
        let mut cursor = Value::Null;
        for _ in 0..100 {
            let v=self.rpc()?.call("thread/list",json!({"limit":100,"cursor":cursor,"archived":archived,"sourceKinds":["cli","vscode","appServer"],"sortKey":"updated_at"})).await?;
            if let Some(a) = v["data"].as_array() {
                for t in a {
                    self.upsert(t, archived);
                }
            }
            cursor = v["nextCursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        Ok(())
    }
    fn upsert(&mut self, t: &Value, archived: bool) -> String {
        let key = text(t, "id");
        if key.is_empty() {
            return key;
        }
        let c = self
            .snap
            .data
            .conversations
            .entry(key.clone())
            .or_insert_with(|| Conversation::new(key.clone(), String::new(), String::new()));
        c.title = t["name"]
            .as_str()
            .filter(|x| !x.is_empty())
            .or_else(|| t["preview"].as_str())
            .unwrap_or("Untitled conversation")
            .chars()
            .take(100)
            .collect();
        if c.title.is_empty() {
            c.title = "Untitled conversation".into()
        }
        if !c.custom_settings {
            c.cwd = text(t, "cwd");
            if let Some(m) = t["model"].as_str() {
                c.model = m.into()
            }
        }
        c.forked_from = t["forkedFromId"].as_str().map(String::from);
        c.archived = archived;
        c.updated = t["updatedAt"].as_u64().unwrap_or(c.updated / 1000) * 1000;
        if let Some(mode) = t["historyMode"].as_str() {
            c.history_mode = mode.into();
        }
        if let Some(status) = t["status"]["type"].as_str() {
            if status == "active" {
                c.status = "running".into()
            } else if !c.busy() && c.status != "unknown" {
                c.status = "idle".into()
            }
        }
        key
    }
    fn begin_load(&mut self, key: &str, older: bool) -> Result<bool> {
        let c = self
            .snap
            .data
            .conversations
            .get(key)
            .context("unknown conversation")?;
        if self.loads.contains_key(key) {
            return Ok(true);
        }
        if !older && c.loaded {
            return Ok(false);
        }
        let older = older && c.loaded;
        let cursor = if older {
            c.history_cursor.clone()
        } else {
            None
        };
        if older && cursor.is_none() {
            return Ok(false);
        }
        let previous_turn = c.last_turn.clone();
        let mode = c.history_mode.clone();
        let rpc = self.rpc()?.handle();
        let events = self.events.clone();
        let generation = self.generation;
        let token = id();
        let load_id = token.clone();
        let key_owned = key.to_owned();
        let task = tokio::spawn(async move {
            let result = async {
                let metadata = if older { None } else {
                    Some(rpc.call("thread/read", json!({"threadId":key_owned,"includeTurns":false})).await?)
                };
                let mode = metadata.as_ref().and_then(|r| r["thread"]["historyMode"].as_str()).unwrap_or(&mode);
                let paginated = mode == "paginated";
                let history = if paginated {
                    rpc.call("thread/turns/list", json!({"threadId":key_owned,"limit":if older {30} else {8},"itemsView":"full","sortDirection":"desc","cursor":cursor})).await?
                } else {
                    rpc.call("thread/read", json!({"threadId":key_owned,"includeTurns":true})).await?
                };
                Ok(LoadedHistory { metadata, history, paginated, older, previous_turn })
            }.await;
            let _ = events.send(json!({"method":"sessiondesk/historyReady","params":{"threadId":key_owned,"loadId":load_id},"_generation":generation}));
            result
        });
        self.loads
            .insert(key.to_owned(), HistoryTask { token, task });
        self.snap.loading.push(key.to_owned());
        self.dirty = true;
        Ok(true)
    }
    async fn finish_load(&mut self, key: &str) -> Result<()> {
        let Some(load) = self.loads.remove(key) else {
            return Ok(());
        };
        let result = load.task.await;
        self.snap.loading.retain(|id| id != key);
        let loaded = result.context("history worker failed")??;
        if let Some(v) = &loaded.metadata {
            let archived = self
                .snap
                .data
                .conversations
                .get(key)
                .is_some_and(|c| c.archived);
            self.upsert(&v["thread"], archived);
            if let Some(c) = self.snap.data.conversations.get_mut(key) {
                if !c.custom_settings {
                    if let Some(model) = v["model"].as_str() {
                        c.model = model.into();
                    }
                    if let Some(p) = v["sandbox"]["type"].as_str() {
                        c.readonly = p == "readOnly";
                    }
                }
            }
        }
        let latest = !loaded.older
            && self
                .snap
                .data
                .conversations
                .get(key)
                .is_some_and(|c| c.last_turn == loaded.previous_turn);
        if loaded.paginated {
            if let Some(a) = loaded.history["data"].as_array() {
                let mut turns = a.clone();
                turns.reverse();
                self.turns(key, &turns, loaded.older, latest);
            }
        } else if let Some(a) = loaded.history["thread"]["turns"].as_array() {
            self.turns(key, a, false, latest);
        }
        if let Some(c) = self.snap.data.conversations.get_mut(key) {
            c.loaded = true;
            c.history_cursor = loaded.history["nextCursor"].as_str().map(String::from);
        }
        Ok(())
    }
    async fn read_history(&mut self, key: &str) -> Result<()> {
        if self.begin_load(key, false)? {
            self.finish_load(key).await?;
        }
        Ok(())
    }
    async fn load(&mut self, key: &str) -> Result<()> {
        self.read_history(key).await?;
        if self
            .snap
            .data
            .conversations
            .get(key)
            .is_some_and(|c| c.resumed)
        {
            return Ok(());
        }
        let v = self
            .rpc()?
            .call("thread/resume", json!({"threadId":key,"excludeTurns":true}))
            .await?;
        self.upsert(&v["thread"], false);
        if let Some(c) = self.snap.data.conversations.get_mut(key) {
            c.resumed = true;
            if !c.custom_settings {
                c.model = text(&v, "model");
                if let Some(p) = v["sandbox"]["type"].as_str() {
                    c.readonly = p == "readOnly";
                }
            }
        }
        Ok(())
    }
    fn turns(&mut self, key: &str, turns: &[Value], prepend: bool, latest: bool) {
        if let Some(c) = self.snap.data.conversations.get_mut(key) {
            let had_completed = c.last_turn.is_some();
            let mut fresh = vec![];
            for turn in turns {
                let tid = text(turn, "id");
                if turn["status"] == "completed" && (latest || !had_completed) {
                    c.last_turn = Some(tid.clone())
                }
                if latest && turn["status"] == "inProgress" {
                    c.active_turn = Some(tid);
                    c.status = "running".into();
                }
                if let Some(items) = turn["items"].as_array() {
                    for item in items {
                        let mid = text(item, "id");
                        let role = role(&text(item, "type"));
                        let content = item_text(item);
                        if let Some(m) = c.messages.iter_mut().find(|m| m.id == mid) {
                            if !content.is_empty()
                                && (m.text.is_empty() || turn["status"] != "inProgress")
                                && (m.text.is_empty()
                                    || item["type"] != "commandExecution"
                                    || item["aggregatedOutput"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty()))
                            {
                                m.text = content;
                            }
                            m.role = role;
                            if turn["status"] != "inProgress" {
                                m.status = item["status"]
                                    .as_str()
                                    .or_else(|| turn["status"].as_str())
                                    .unwrap_or("completed")
                                    .into();
                            }
                        } else {
                            fresh.push(Message {
                                id: mid,
                                role,
                                text: content,
                                status: item["status"]
                                    .as_str()
                                    .or_else(|| turn["status"].as_str())
                                    .unwrap_or_default()
                                    .into(),
                            });
                        }
                    }
                }
            }
            if prepend {
                fresh.append(&mut c.messages);
                c.messages = fresh
            } else {
                c.messages.extend(fresh)
            }
        }
    }
    async fn command(&mut self, op: &str, a: Value) -> Result<Value> {
        let raw_key = text(&a, "thread");
        let key = self
            .snap
            .data
            .aliases
            .get(&raw_key)
            .cloned()
            .unwrap_or(raw_key);
        match op {
            "search" => {
                let query = text(&a, "query").trim().to_owned();
                anyhow::ensure!(!query.is_empty(), "Enter message text to search");
                anyhow::ensure!(
                    query.chars().count() <= 256,
                    "Search is limited to 256 characters"
                );
                let rpc = self.rpc()?.handle();
                if let Some(cancel) = self.search_cancel.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                let job = id();
                let cancel = Arc::new(AtomicBool::new(false));
                self.snap.search = SearchState {
                    job: job.clone(),
                    query: query.clone(),
                    running: true,
                    ..Default::default()
                };
                crate::search::start(
                    rpc,
                    self.events.clone(),
                    self.generation,
                    self.snap.data.conversations.clone(),
                    job.clone(),
                    query,
                    cancel.clone(),
                );
                self.search_cancel = Some(cancel);
                Ok(json!({"job":job}))
            }
            "search_cancel" => {
                if text(&a, "job").is_empty() || text(&a, "job") == self.snap.search.job {
                    if let Some(cancel) = self.search_cancel.take() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    self.snap.search = SearchState::default();
                }
                Ok(json!({}))
            }
            "search_context" => {
                let mid = text(&a, "message");
                let thread = key.clone();
                let context =
                    tokio::task::spawn_blocking(move || crate::search::context(&thread, &mid))
                        .await??;
                self.snap.data.conversations.entry(key).or_insert_with(|| {
                    let mut c = Conversation::new(
                        context.hit.thread.clone(),
                        context.hit.title.clone(),
                        context.hit.cwd.clone(),
                    );
                    c.archived = context.hit.archived;
                    c
                });
                Ok(serde_json::to_value(context)?)
            }
            "prepare" => {
                let cwd = absolute(&text(&a, "cwd"))?;
                let key = format!("local-{}", id());
                let mut c = Conversation::new(key.clone(), text(&a, "title"), cwd);
                if c.title.is_empty() {
                    c.title = "New conversation".into()
                }
                c.model = text(&a, "model");
                c.readonly = a["readonly"].as_bool().unwrap_or(false);
                c.loaded = true;
                self.snap.data.conversations.insert(key.clone(), c);
                Ok(json!({"thread":key}))
            }
            "snapshot" => Ok(serde_json::to_value(&self.snap)?),
            "refresh" => {
                self.refresh(a["archived"].as_bool().unwrap_or(false))
                    .await?;
                Ok(json!({}))
            }
            "reconnect" => {
                self.connect().await?;
                Ok(json!({}))
            }
            "open" => {
                let loading = !key.starts_with("local-") && self.begin_load(&key, false)?;
                if let Some(c) = self.snap.data.conversations.get_mut(&key) {
                    c.unread = false;
                }
                Ok(json!({"thread":key,"loading":loading}))
            }
            "read" => {
                if let Some(c) = self.snap.data.conversations.get_mut(&key) {
                    c.unread = false;
                }
                Ok(json!({}))
            }
            "scroll" => {
                self.snap
                    .data
                    .scroll
                    .insert(key, a["offset"].as_u64().unwrap_or(0).min(65535) as u16);
                Ok(json!({}))
            }
            "history" => {
                let loading = self.begin_load(&key, true)?;
                Ok(
                    json!({"loading":loading,"message":if loading {"Loading history in the background…"} else {"All available history is loaded."}}),
                )
            }
            "new" => {
                let cwd = absolute(&text(&a, "cwd"))?;
                let readonly = a["readonly"].as_bool().unwrap_or(false);
                let mut p = json!({"cwd":cwd,"approvalPolicy":"on-request","sandbox":if readonly{"read-only"}else{"workspace-write"}});
                if !text(&a, "model").is_empty() {
                    p["model"] = a["model"].clone()
                }
                let v = self.rpc()?.call("thread/start", p).await?;
                let key = self.upsert(&v["thread"], false);
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("backend returned no thread")?;
                c.loaded = true;
                c.resumed = true;
                c.readonly = readonly;
                c.custom_settings = true;
                let title = text(&a, "title");
                if !title.is_empty() {
                    self.rpc()?
                        .call("thread/name/set", json!({"threadId":key,"name":title}))
                        .await?;
                    self.snap.data.conversations.get_mut(&key).unwrap().title = title;
                }
                self.snap.data.active = Some(key.clone());
                Ok(json!({"thread":key}))
            }
            "side" => {
                self.read_history(&key).await?;
                let parent = self
                    .snap
                    .data
                    .conversations
                    .get(&key)
                    .context("select a parent conversation")?
                    .clone();
                let v = if let Some(last) = &parent.last_turn {
                    self.rpc()?.call("thread/fork",json!({"threadId":key,"lastTurnId":last,"excludeTurns":true,"ephemeral":false,"sandbox":"read-only"})).await?
                } else {
                    self.rpc()?.call("thread/start",json!({"cwd":parent.cwd,"model":if parent.model.is_empty(){Value::Null}else{json!(parent.model)},"sandbox":"read-only","approvalPolicy":"on-request"})).await?
                };
                let child = self.upsert(&v["thread"], false);
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&child)
                    .context("missing side thread")?;
                c.parent = Some(key.clone());
                c.project = parent.project.clone();
                c.model = parent.model.clone();
                c.cwd = parent.cwd.clone();
                c.readonly = true;
                c.custom_settings = true;
                c.loaded = false;
                c.title = format!("Side · {}", parent.title);
                if parent.last_turn.is_none() {
                    if let Some(m) = parent.messages.iter().find(|m| m.role == "You") {
                        self.snap.data.drafts.insert(
                            child.clone(),
                            Draft {
                                text: format!("Parent request (quoted context):\n{}\n\n", m.text),
                                cursor: 0,
                            },
                        );
                    }
                }
                let title = self.snap.data.conversations[&child].title.clone();
                self.rpc()?
                    .call("thread/name/set", json!({"threadId":child,"name":title}))
                    .await?;
                self.read_history(&child).await?;
                Ok(json!({"thread":child,"parent":key,"boundary":parent.last_turn}))
            }
            "send" => {
                if text(&a, "text").trim().is_empty() {
                    return Err(anyhow!("message is empty"));
                }
                let old_key = key.clone();
                let key = if key.starts_with("local-") {
                    self.materialize(&key).await?
                } else {
                    key
                };
                self.load(&key).await?;
                if a["steer"] == true {
                    let c = self
                        .snap
                        .data
                        .conversations
                        .get(&key)
                        .context("unknown thread")?;
                    let turn = c.active_turn.clone().context("no active turn to steer")?;
                    self.rpc()?.call("turn/steer",json!({"threadId":key,"expectedTurnId":turn,"input":[{"type":"text","text":a["text"]}]})).await?;
                } else {
                    let c = self
                        .snap
                        .data
                        .conversations
                        .get_mut(&key)
                        .context("unknown thread")?;
                    c.queued.push(Queued {
                        id: id(),
                        text: text(&a, "text"),
                        cwd: c.cwd.clone(),
                        readonly: c.readonly,
                        state: "queued".into(),
                    });
                    if !c.busy() {
                        c.status = "queued".into();
                    }
                }
                self.snap.data.drafts.remove(&key);
                self.snap.data.drafts.remove(&old_key);
                Ok(json!({"thread":key,"replace":old_key}))
            }
            "stop" => {
                let turn = self
                    .snap
                    .data
                    .conversations
                    .get(&key)
                    .and_then(|c| c.active_turn.clone())
                    .context("no active task")?;
                self.rpc()?
                    .call("turn/interrupt", json!({"threadId":key,"turnId":turn}))
                    .await?;
                self.snap.data.conversations.get_mut(&key).unwrap().status = "stopping".into();
                Ok(json!({}))
            }
            "resolve" => {
                let request = self
                    .snap
                    .pending
                    .iter()
                    .find(|p| p.id == a["requestId"])
                    .context("request was resolved or backend connection changed")?
                    .clone();
                anyhow::ensure!(
                    a["generation"].as_u64() == Some(request.generation)
                        && a["thread"] == request.thread,
                    "request belongs to another backend connection or conversation"
                );
                self.rpc()?
                    .respond(request.id.clone(), a["result"].clone())?;
                self.snap.pending.retain(|p| p.id != request.id);
                if !self.snap.pending.iter().any(|p| p.thread == request.thread) {
                    if let Some(c) = self.snap.data.conversations.get_mut(&request.thread) {
                        c.status = if c.active_turn.is_some() {
                            "running"
                        } else {
                            "idle"
                        }
                        .into();
                    }
                }
                Ok(json!({}))
            }
            "draft" => {
                self.snap
                    .data
                    .drafts
                    .insert(key, serde_json::from_value(a["draft"].clone())?);
                Ok(json!({}))
            }
            "view" => {
                let root = text(&a, "root");
                let layout: Node = serde_json::from_value(a["layout"].clone())?;
                if layout.leaves() > 4 {
                    return Err(anyhow!("maximum four visible panes"));
                }
                self.snap.data.layouts.insert(root.clone(), layout);
                self.snap.data.active = Some(root);
                self.snap.data.sidebar = a["sidebar"].as_bool().unwrap_or(true);
                self.snap.data.sidebar_width =
                    a["width"].as_u64().unwrap_or(28).clamp(18, 52) as u16;
                if let Some(dirs) = a.get("collapsedDirectories") {
                    self.snap.data.collapsed_dirs = serde_json::from_value(dirs.clone())?;
                }
                if let Some(roots) = a.get("collapsedSidePanels") {
                    self.snap.data.collapsed_side_panels = serde_json::from_value(roots.clone())?;
                }
                Ok(json!({}))
            }
            "pin" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown conversation")?;
                c.pinned = !c.pinned;
                Ok(json!({}))
            }
            "rename" => {
                if key.starts_with("local-") {
                    self.snap
                        .data
                        .conversations
                        .get_mut(&key)
                        .context("unknown conversation")?
                        .title = text(&a, "title");
                    return Ok(json!({}));
                }
                self.rpc()?
                    .call("thread/name/set", json!({"threadId":key,"name":a["title"]}))
                    .await?;
                self.snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown conversation")?
                    .title = text(&a, "title");
                Ok(json!({}))
            }
            "settings" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown conversation")?;
                if c.busy() || !c.queued.is_empty() {
                    return Err(anyhow!("stop the task and clear queued messages before changing execution settings"));
                }
                c.custom_settings = true;
                if let Some(m) = a["model"].as_str() {
                    c.model = m.into()
                }
                if let Some(ro) = a["readonly"].as_bool() {
                    c.readonly = ro
                }
                if let Some(cwd) = a["cwd"].as_str() {
                    c.cwd = absolute(cwd)?;
                }
                Ok(json!({}))
            }
            "queue" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown conversation")?;
                if a["clear"] == true {
                    c.queued.clear()
                } else {
                    for q in &mut c.queued {
                        q.state = "queued".into();
                    }
                }
                Ok(json!({}))
            }
            "project" => {
                let name = text(&a, "name");
                if name.trim().is_empty() {
                    return Err(anyhow!("project name is required"));
                }
                if let Some(p) = self.snap.data.projects.iter_mut().find(|p| p.name == name) {
                    if let Some(cwd) = a["cwd"].as_str() {
                        let path = absolute(cwd)?;
                        if !p.roots.contains(&path) {
                            p.roots.push(path)
                        }
                    }
                } else {
                    let roots = if let Some(cwd) = a["cwd"].as_str() {
                        vec![absolute(cwd)?]
                    } else {
                        vec![]
                    };
                    self.snap.data.projects.push(Project {
                        id: id(),
                        name: name.clone(),
                        roots,
                    });
                }
                if let Some(c) = self.snap.data.conversations.get_mut(&key) {
                    c.project = Some(name);
                }
                Ok(json!({}))
            }
            "reorder" => {
                let source = text(&a, "source");
                let target = text(&a, "target");
                self.snap.data.order.retain(|x| x != &source);
                let i = self
                    .snap
                    .data
                    .order
                    .iter()
                    .position(|x| x == &target)
                    .unwrap_or(self.snap.data.order.len());
                self.snap.data.order.insert(i, source);
                Ok(json!({}))
            }
            "promote" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown side")?;
                c.parent = None;
                c.promoted = true;
                Ok(json!({}))
            }
            "archive" => {
                if key.starts_with("local-") {
                    self.snap
                        .data
                        .conversations
                        .get_mut(&key)
                        .context("unknown conversation")?
                        .archived = true;
                    return Ok(json!({}));
                }
                let c = self
                    .snap
                    .data
                    .conversations
                    .get(&key)
                    .context("unknown conversation")?;
                if c.busy() {
                    return Err(anyhow!("stop the active task before archiving"));
                }
                if !c.queued.is_empty() {
                    return Err(anyhow!("clear queued messages before archiving"));
                }
                if self.snap.data.conversations.values().any(|c| {
                    (c.parent.as_deref() == Some(&key) || c.forked_from.as_deref() == Some(&key))
                        && !c.archived
                }) {
                    return Err(anyhow!("This thread has forks. Archive descendants explicitly first; the backend may archive the entire tree."));
                }
                self.rpc()?
                    .call("thread/archive", json!({"threadId":key}))
                    .await?;
                self.snap.data.conversations.get_mut(&key).unwrap().archived = true;
                Ok(json!({}))
            }
            "restore" => {
                if key.starts_with("local-") {
                    self.snap
                        .data
                        .conversations
                        .get_mut(&key)
                        .context("unknown conversation")?
                        .archived = false;
                    return Ok(json!({}));
                }
                self.rpc()?
                    .call("thread/unarchive", json!({"threadId":key}))
                    .await?;
                self.snap
                    .data
                    .conversations
                    .get_mut(&key)
                    .context("unknown conversation")?
                    .archived = false;
                Ok(json!({}))
            }
            "diff" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get(&key)
                    .context("unknown conversation")?;
                let out = tokio::process::Command::new("git")
                    .args(["diff", "--no-ext-diff", "--no-color", "HEAD", "--"])
                    .current_dir(&c.cwd)
                    .output()
                    .await?;
                if !out.status.success() {
                    return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
                }
                Ok(
                    json!({"diff":String::from_utf8_lossy(&out.stdout),"label":"Workspace diff · includes changes from all sources"}),
                )
            }
            "worktree" => {
                let c = self
                    .snap
                    .data
                    .conversations
                    .get(&key)
                    .context("select a repository conversation")?
                    .clone();
                let path = PathBuf::from(text(&a, "path"));
                if !path.is_absolute() {
                    return Err(anyhow!("worktree path must be absolute"));
                }
                let base = text(&a, "base");
                if base.is_empty() || base.starts_with('-') {
                    return Err(anyhow!("provide an existing base ref"));
                }
                let out = tokio::process::Command::new("git")
                    .args(["worktree", "add", "--detach"])
                    .arg(&path)
                    .arg(&base)
                    .current_dir(&c.cwd)
                    .output()
                    .await?;
                if !out.status.success() {
                    return Err(anyhow!(String::from_utf8_lossy(&out.stderr).to_string()));
                }
                Ok(
                    json!({"path":path,"message":"Worktree created. Use New conversation with this directory; cleanup is manual."}),
                )
            }
            "login" => {
                self.rpc()?
                    .call("account/login/start", json!({"type":"chatgptDeviceCode"}))
                    .await
            }
            "shutdown" => {
                if self
                    .snap
                    .data
                    .conversations
                    .values()
                    .any(Conversation::busy)
                {
                    return Err(anyhow!(
                        "tasks are active; stop them before shutting down the service"
                    ));
                }
                Ok(json!({}))
            }
            _ => Err(anyhow!("unknown operation: {op}")),
        }
    }
    async fn materialize(&mut self, old: &str) -> Result<String> {
        let source = self
            .snap
            .data
            .conversations
            .get(old)
            .context("missing local draft")?
            .clone();
        let mut p = json!({"cwd":source.cwd,"sandbox":if source.readonly{"read-only"}else{"workspace-write"},"approvalPolicy":"on-request"});
        if !source.model.is_empty() {
            p["model"] = json!(source.model)
        }
        let v = self.rpc()?.call("thread/start", p).await?;
        let key = self.upsert(&v["thread"], false);
        let c = self
            .snap
            .data
            .conversations
            .get_mut(&key)
            .context("missing thread")?;
        c.loaded = true;
        c.resumed = true;
        c.model = if source.model.is_empty() {
            text(&v, "model")
        } else {
            source.model
        };
        c.readonly = source.readonly;
        c.custom_settings = true;
        c.project = source.project;
        c.title = source.title.clone();
        // Move identity immediately: a later send error must never create another backend thread.
        self.snap.data.conversations.remove(old);
        self.snap.data.aliases.insert(old.into(), key.clone());
        if let Some(d) = self.snap.data.drafts.remove(old) {
            self.snap.data.drafts.insert(key.clone(), d);
        }
        if let Some(mut layout) = self.snap.data.layouts.remove(old) {
            layout.replace(old, &key);
            self.snap.data.layouts.insert(key.clone(), layout);
        }
        for layout in self.snap.data.layouts.values_mut() {
            layout.replace(old, &key);
        }
        self.snap.data.active = Some(key.clone());
        if source.title != "New conversation" {
            self.rpc()?
                .call(
                    "thread/name/set",
                    json!({"threadId":key,"name":source.title}),
                )
                .await?;
        }
        Ok(key)
    }
    async fn schedule(&mut self) {
        loop {
            for c in self.snap.data.conversations.values() {
                self.keys
                    .entry(c.cwd.clone())
                    .or_insert_with(|| workspace(&c.cwd));
            }
            let active: Vec<_> = self
                .snap
                .data
                .conversations
                .values()
                .filter(|c| c.busy())
                .cloned()
                .collect();
            if active.len() >= 4 {
                break;
            }
            let candidate = self
                .snap
                .data
                .conversations
                .values()
                .find(|c| {
                    !c.busy()
                        && c.status != "unknown"
                        && c.queued.first().is_some_and(|q| q.state == "queued")
                        && (c.readonly
                            || !active.iter().any(|other| {
                                !other.readonly && {
                                    let a = &self.keys[&c.cwd];
                                    let b = &self.keys[&other.cwd];
                                    a.starts_with(b) || b.starts_with(a)
                                }
                            }))
                })
                .map(|c| c.id.clone());
            let Some(key) = candidate else { break };
            let c = self.snap.data.conversations.get_mut(&key).unwrap();
            let q = c.queued.remove(0);
            if q.cwd != c.cwd || q.readonly != c.readonly {
                c.queued.insert(
                    0,
                    Queued {
                        state: "held".into(),
                        ..q
                    },
                );
                continue;
            }
            c.status = "starting".into();
            let model = c.model.clone();
            let mut params = json!({"threadId":key,"cwd":q.cwd,"input":[{"type":"text","text":q.text}],"summary":"auto","sandboxPolicy":{"type":if q.readonly{"readOnly"}else{"workspaceWrite"}}});
            if !model.is_empty() {
                params["model"] = json!(model)
            }
            let result = match self.rpc() {
                Ok(r) => r.call("turn/start", params).await,
                Err(e) => Err(e),
            };
            let c = self.snap.data.conversations.get_mut(&key).unwrap();
            match result {
                Ok(v) => {
                    c.active_turn = v["turn"]["id"].as_str().map(String::from);
                    c.status = "running".into();
                    c.updated = now();
                }
                Err(e) => {
                    c.status = "unknown".into();
                    c.queued.insert(
                        0,
                        Queued {
                            state: "uncertain".into(),
                            ..q
                        },
                    );
                    self.snap.error = e.to_string();
                    break;
                }
            }
            self.dirty = true;
        }
    }
    fn event(&mut self, v: Value) {
        if v["_generation"]
            .as_u64()
            .is_some_and(|g| g != self.generation)
        {
            return;
        }
        let method = text(&v, "method");
        let p = &v["params"];
        let key = p["threadId"].as_str().unwrap_or_default().to_string();
        if method == "sessiondesk/disconnected" {
            self.snap.connected = false;
            self.snap.error="Codex disconnected. Use /reconnect; uncertain tasks are never automatically resent.".into();
            self.snap.pending.clear();
            self.activity.clear();
            for c in self.snap.data.conversations.values_mut() {
                if c.busy() {
                    c.status = "unknown".into();
                    c.active_turn = None;
                }
                c.loaded = false;
                c.resumed = false;
            }
            return;
        }
        if v.get("id").is_some() {
            self.snap.pending.push(Pending {
                id: v["id"].clone(),
                method: method.clone(),
                params: p.clone(),
                thread: key.clone(),
                generation: self.generation,
            });
            if let Some(c) = self.snap.data.conversations.get_mut(&key) {
                c.status = if method.contains("Approval") {
                    "waitingApproval"
                } else {
                    "waitingInput"
                }
                .into();
            }
            return;
        }
        if method == "serverRequest/resolved" {
            self.snap.pending.retain(|r| r.id != p["requestId"]);
            return;
        }
        if method == "thread/started" {
            self.upsert(&p["thread"], false);
            return;
        }
        if method == "thread/name/updated" {
            if let Some(c) = self.snap.data.conversations.get_mut(&key) {
                c.title = text(p, "threadName");
                if c.title.is_empty() {
                    c.title = text(p, "name")
                }
            }
            return;
        }
        let Some(c) = self.snap.data.conversations.get_mut(&key) else {
            return;
        };
        c.unread = true;
        c.updated = now();
        if self.activity.update(c, &method, p) {
            if c.messages.len() > 1500 {
                c.messages.drain(..c.messages.len() - 1000);
            }
            return;
        }
        match method.as_str() {
            "turn/started" => {
                c.active_turn = p["turn"]["id"].as_str().map(String::from);
                c.status = "running".into();
            }
            "turn/completed" => {
                c.status = match p["turn"]["status"].as_str().unwrap_or("completed") {
                    "failed" => "failed",
                    "interrupted" => "interrupted",
                    _ => "completed",
                }
                .into();
                if c.status == "completed" {
                    c.last_turn = p["turn"]["id"].as_str().map(String::from)
                }
                c.active_turn = None;
                let status = c.status.clone();
                self.activity.finish(c, &text(&p["turn"], "id"), &status);
                self.snap.pending.retain(|r| r.thread != key);
                if let Some(error) = p["turn"]["error"].as_object() {
                    c.item(&id(), "Error").text = serde_json::to_string(error).unwrap_or_default();
                }
            }
            "turn/diff/updated" => c.diff = text(p, "diff"),
            "thread/status/changed" => {
                let status = &p["status"];
                if status["type"] == "active" {
                    c.status = if status["activeFlags"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == "waitingOnApproval"))
                    {
                        "waitingApproval"
                    } else if status["activeFlags"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == "waitingOnUserInput"))
                    {
                        "waitingInput"
                    } else {
                        "running"
                    }
                    .into();
                }
            }
            "error" => {
                c.item(&id(), "Error").text = p.to_string();
            }
            _ => {}
        }
        // Keep only the recent window in memory; older history remains owned by Codex.
        if c.messages.len() > 1500 {
            c.messages.drain(..c.messages.len() - 1000);
        }
    }
}
fn absolute(s: &str) -> Result<String> {
    let p = Path::new(s);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };
    let p = p
        .canonicalize()
        .with_context(|| format!("directory does not exist: {}", p.display()))?;
    anyhow::ensure!(p.is_dir(), "not a directory: {}", p.display());
    Ok(p.to_string_lossy().into_owned())
}
fn workspace(s: &str) -> PathBuf {
    let p = Path::new(s)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(s));
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&p)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or(p)
}
