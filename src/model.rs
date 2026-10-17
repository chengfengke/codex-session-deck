use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn id() -> String {
    format!(
        "{}-{}",
        now(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub cwd: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub last_turn: Option<String>,
    #[serde(default)]
    pub active_turn: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub updated: u64,
    #[serde(default)]
    pub diff: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub queued: Vec<Queued>,
    #[serde(default)]
    pub loaded: bool,
    #[serde(default)]
    pub resumed: bool,
    #[serde(default)]
    pub history_cursor: Option<String>,
    #[serde(default)]
    pub history_mode: String,
    #[serde(default)]
    pub promoted: bool,
    #[serde(default)]
    pub forked_from: Option<String>,
    #[serde(default)]
    pub custom_settings: bool,
}
impl Conversation {
    pub fn new(key: String, title: String, cwd: String) -> Self {
        Self {
            id: key,
            title,
            cwd,
            status: "idle".into(),
            model: String::new(),
            messages: vec![],
            last_turn: None,
            active_turn: None,
            parent: None,
            pinned: false,
            archived: false,
            unread: false,
            readonly: false,
            updated: now(),
            diff: String::new(),
            project: None,
            queued: vec![],
            loaded: false,
            resumed: false,
            history_cursor: None,
            history_mode: String::new(),
            promoted: false,
            forked_from: None,
            custom_settings: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.active_turn.is_some()
            || matches!(
                self.status.as_str(),
                "running" | "waitingApproval" | "waitingInput" | "stopping" | "starting"
            )
    }
    pub fn item(&mut self, key: &str, role: &str) -> &mut Message {
        if let Some(i) = self.messages.iter().position(|m| m.id == key) {
            return &mut self.messages[i];
        }
        self.messages.push(Message {
            id: key.into(),
            role: role.into(),
            ..Default::default()
        });
        self.messages.last_mut().unwrap()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Queued {
    pub id: String,
    pub text: String,
    pub cwd: String,
    pub readonly: bool,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub id: Value,
    pub method: String,
    pub params: Value,
    pub thread: String,
    #[serde(default)]
    pub generation: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub roots: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    pub cursor: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Persisted {
    pub conversations: BTreeMap<String, Conversation>,
    pub projects: Vec<Project>,
    pub drafts: BTreeMap<String, Draft>,
    pub layouts: BTreeMap<String, crate::layout::Node>,
    pub active: Option<String>,
    pub sidebar: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u16,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub scroll: BTreeMap<String, u16>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub collapsed_dirs: Vec<String>,
    #[serde(default)]
    pub collapsed_side_panels: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchHit {
    pub thread: String,
    pub message: String,
    pub title: String,
    pub cwd: String,
    pub role: String,
    pub excerpt: String,
    pub archived: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchState {
    pub job: String,
    pub query: String,
    pub running: bool,
    pub completed: usize,
    pub total: usize,
    pub hits: Vec<SearchHit>,
    pub errors: usize,
    pub error: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchContext {
    pub hit: SearchHit,
    pub messages: Vec<Message>,
}
fn default_sidebar_width() -> u16 {
    28
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub data: Persisted,
    pub pending: Vec<Pending>,
    pub models: Vec<Value>,
    pub connected: bool,
    pub error: String,
    pub seq: u64,
    pub account: String,
    #[serde(default)]
    pub loading: Vec<String>,
    #[serde(default)]
    pub search: SearchState,
}

#[derive(Serialize, Deserialize)]
pub struct HistoryDelta {
    pub keep: usize,
    pub append: Vec<Message>,
}
#[derive(Serialize, Deserialize)]
pub struct SnapshotDelta {
    pub metadata: Snapshot,
    pub histories: BTreeMap<String, HistoryDelta>,
}
impl SnapshotDelta {
    pub fn between(old: &Snapshot, new: &Snapshot) -> Self {
        let mut metadata = new.clone();
        let mut histories = BTreeMap::new();
        for (key, c) in &mut metadata.data.conversations {
            let messages = std::mem::take(&mut c.messages);
            let previous = old
                .data
                .conversations
                .get(key)
                .map(|c| c.messages.as_slice())
                .unwrap_or_default();
            let keep = previous
                .iter()
                .zip(&messages)
                .take_while(|(a, b)| a == b)
                .count();
            if keep != previous.len() || keep != messages.len() {
                histories.insert(
                    key.clone(),
                    HistoryDelta {
                        keep,
                        append: messages.into_iter().skip(keep).collect(),
                    },
                );
            }
        }
        Self {
            metadata,
            histories,
        }
    }
    pub fn apply(mut self, old: &mut Snapshot) -> Snapshot {
        for (key, c) in &mut self.metadata.data.conversations {
            let mut messages = old
                .data
                .conversations
                .get_mut(key)
                .map(|c| std::mem::take(&mut c.messages))
                .unwrap_or_default();
            if let Some(delta) = self.histories.remove(key) {
                messages.truncate(delta.keep);
                messages.extend(delta.append);
            }
            c.messages = messages;
        }
        self.metadata
    }
}
pub fn state_dir() -> PathBuf {
    if let Some(p) =
        std::env::var_os("CODEX_DECK_HOME").or_else(|| std::env::var_os("SESSIONDESK_HOME"))
    {
        return p.into();
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        });
    let profile = codex_home.canonicalize().unwrap_or(codex_home);
    let hash = profile
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
    // Keep the original location so renaming never splits an existing workspace.
    base.join("sessiondesk").join(format!("{hash:016x}"))
}
pub fn socket_path() -> PathBuf {
    state_dir().join("runtime.sock")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delta_preserves_history_through_append_edit_prepend_and_removal() {
        let mut previous = Snapshot::default();
        let mut c = Conversation::new(
            "thread".into(),
            "History".into(),
            std::env::temp_dir().to_string_lossy().into_owned(),
        );
        c.messages.push(Message {
            id: "old".into(),
            text: "unchanged history".repeat(10000),
            ..Default::default()
        });
        previous.data.conversations.insert(c.id.clone(), c);
        let mut next = previous.clone();
        next.data.drafts.insert(
            "thread".into(),
            Draft {
                text: "draft".into(),
                cursor: 5,
            },
        );
        let delta = SnapshotDelta::between(&previous, &next);
        assert!(delta.histories.is_empty());
        assert!(serde_json::to_vec(&delta).unwrap().len() < 3000);
        for action in 0..5 {
            let c = next.data.conversations.get_mut("thread").unwrap();
            match action {
                0 => c.messages.push(Message {
                    id: "new".into(),
                    text: "stream".into(),
                    ..Default::default()
                }),
                1 => c.messages.last_mut().unwrap().text.push_str(" delta"),
                2 => c.messages.insert(
                    0,
                    Message {
                        id: "older".into(),
                        text: "paged history".into(),
                        ..Default::default()
                    },
                ),
                3 => {
                    c.messages.remove(1);
                }
                _ => c.messages.clear(),
            }
            let delta = SnapshotDelta::between(&previous, &next);
            previous = delta.apply(&mut previous);
            assert_eq!(
                serde_json::to_value(&previous).unwrap(),
                serde_json::to_value(&next).unwrap()
            );
        }
        next.data.conversations.clear();
        previous = SnapshotDelta::between(&previous, &next).apply(&mut previous);
        assert!(previous.data.conversations.is_empty());
    }
}
