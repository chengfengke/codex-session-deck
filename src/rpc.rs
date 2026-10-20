use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot, Mutex},
};

type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
pub struct Rpc {
    handle: RpcHandle,
    child: Child,
}
#[derive(Clone)]
pub struct RpcHandle {
    writer: mpsc::UnboundedSender<Value>,
    pending: Waiters,
    next: Arc<AtomicU64>,
}
impl std::ops::Deref for Rpc {
    type Target = RpcHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl Rpc {
    pub async fn start(events: mpsc::UnboundedSender<Value>, generation: u64) -> Result<Self> {
        let exe = std::env::var("CODEX_DECK_CODEX")
            .or_else(|_| std::env::var("SESSIONDESK_CODEX"))
            .unwrap_or_else(|_| "codex".into());
        let mut child = Command::new(&exe)
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!("Cannot start {exe}; install Codex CLI or set CODEX_DECK_CODEX")
            })?;
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let stdout = child.stdout.take().context("missing stdout")?;
        let (writer, mut rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            while let Some(v) = rx.recv().await {
                let mut b = serde_json::to_vec(&v).unwrap();
                b.push(b'\n');
                if stdin.write_all(&b).await.is_err() {
                    break;
                }
            }
        });
        let pending: Waiters = Arc::new(Mutex::new(HashMap::new()));
        let p = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(mut v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if v.get("method").is_none() {
                    if let Some(id) = v["id"].as_u64() {
                        if let Some(tx) = p.lock().await.remove(&id) {
                            let _ = tx.send(if v.get("error").is_some() {
                                Err(v["error"].to_string())
                            } else {
                                Ok(v["result"].clone())
                            });
                        }
                    }
                } else {
                    v["_generation"] = json!(generation);
                    let _ = events.send(v);
                }
            }
            let mut p = p.lock().await;
            for (_, tx) in p.drain() {
                let _ = tx.send(Err(
                    "Codex app-server disconnected; submission may be uncertain".into(),
                ));
            }
            let _ =
                events.send(json!({"method":"sessiondesk/disconnected","_generation":generation}));
        });
        Ok(Self {
            handle: RpcHandle {
                writer,
                pending,
                next: Arc::new(AtomicU64::new(1)),
            },
            child,
        })
    }
    pub fn handle(&self) -> RpcHandle {
        self.handle.clone()
    }
    pub async fn stop(&mut self) {
        let _ = self.child.kill().await;
    }
}
impl RpcHandle {
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.writer
            .send(json!({"id":id,"method":method,"params":params}))
            .map_err(|_| anyhow!("backend disconnected"))?;
        match tokio::time::timeout(std::time::Duration::from_secs(45), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            _ => {
                self.pending.lock().await.remove(&id);
                Err(anyhow!(
                    "Codex request timed out; do not automatically resend a submitted task"
                ))
            }
        }
    }
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.writer
            .send(json!({"method":method,"params":params}))
            .map_err(|_| anyhow!("backend disconnected"))
    }
    pub fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.writer
            .send(json!({"id":id,"result":result}))
            .map_err(|_| anyhow!("backend disconnected"))
    }
}

pub fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}
pub use crate::activity::item_text;
