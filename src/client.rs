use crate::model::{socket_path, state_dir};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{os::unix::process::CommandExt, process::Stdio, sync::mpsc as sync};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};

pub struct Client {
    pub tx: mpsc::UnboundedSender<Value>,
    pub rx: sync::Receiver<Value>,
    next: u64,
}
impl Client {
    #[cfg(test)]
    pub fn fixture() -> (Self, sync::Sender<Value>, mpsc::UnboundedReceiver<Value>) {
        let (tx, out) = mpsc::unbounded_channel();
        let (input, rx) = sync::channel();
        (Self { tx, rx, next: 1 }, input, out)
    }
    pub async fn connect() -> Result<Self> {
        Self::connect_mode(true).await
    }
    pub async fn attach() -> Result<Self> {
        Self::connect_mode(false).await
    }
    async fn connect_mode(start: bool) -> Result<Self> {
        let stream = match UnixStream::connect(socket_path()).await {
            Ok(s) => s,
            Err(e) => {
                if !start {
                    return Err(e).context("Background service is not running");
                }
                std::fs::create_dir_all(state_dir())?;
                let log = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(state_dir().join("daemon.log"))?;
                let mut cmd = std::process::Command::new(std::env::current_exe()?);
                cmd.arg("daemon")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(log);
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setsid() < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                cmd.spawn().context("cannot launch background service")?;
                let mut found = None;
                for _ in 0..100 {
                    if let Ok(s) = UnixStream::connect(socket_path()).await {
                        found = Some(s);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                found.context("daemon did not start; inspect daemon.log in CODEX_DECK_HOME (or legacy SESSIONDESK_HOME)")?
            }
        };
        let (read, mut write) = stream.into_split();
        let (tx, mut outgoing) = mpsc::unbounded_channel::<Value>();
        let (incoming, rx) = sync::channel();
        tx.send(json!({"id":0,"op":"subscribe","args":{"delta":true}}))?;
        tokio::spawn(async move {
            while let Some(v) = outgoing.recv().await {
                let mut b = serde_json::to_vec(&v).unwrap();
                b.push(b'\n');
                if write.write_all(&b).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str(&line) {
                    if incoming.send(v).is_err() {
                        return;
                    }
                }
            }
            let _ = incoming.send(json!({"type":"disconnect"}));
        });
        Ok(Self { tx, rx, next: 1 })
    }
    pub fn send(&mut self, op: &str, args: Value) -> u64 {
        self.next += 1;
        let _ = self.tx.send(json!({"id":self.next,"op":op,"args":args}));
        self.next
    }
}
