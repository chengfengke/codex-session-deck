use crate::{
    client::Client,
    layout::{Divider, Node, Pane},
    model::*,
    rpc::text,
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, IsTerminal},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const BG: Color = Color::Rgb(15, 19, 27);
const PANEL: Color = Color::Rgb(21, 27, 37);
const FG: Color = Color::Rgb(218, 225, 235);
const MUTED: Color = Color::Rgb(125, 143, 165);
const ACCENT: Color = Color::Rgb(98, 214, 199);
const BORDER: Color = Color::Rgb(49, 63, 81);
const SELECT: Color = Color::Rgb(34, 60, 72);
fn style(c: Color) -> Style {
    Style::default().fg(c)
}
fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}
fn short(s: &str, n: usize) -> String {
    if s.width() <= n {
        return s.into();
    }
    let mut out = String::new();
    for g in s.graphemes(true) {
        if out.width() + g.width() + 1 > n {
            break;
        }
        out.push_str(g)
    }
    out.push('…');
    out
}
fn label(c: &Conversation) -> String {
    let icon = if c.busy() {
        if c.status.starts_with("waiting") {
            "!"
        } else {
            "●"
        }
    } else if !c.queued.is_empty() {
        "◷"
    } else if c.unread {
        "•"
    } else {
        " "
    };
    format!("{}{} {}", if c.pinned { "★" } else { " " }, icon, c.title)
}

#[derive(Clone)]
enum Modal {
    Find {
        query: String,
        submitted: String,
        selected: usize,
    },
    Tasks {
        query: String,
        selected: usize,
        filter: usize,
    },
    Models {
        thread: String,
        query: String,
        selected: usize,
    },
    Search {
        query: String,
        selected: usize,
        archived: bool,
    },
    New {
        fields: Vec<String>,
        focus: usize,
        readonly: bool,
    },
    Prompt {
        title: String,
        value: String,
        action: String,
    },
    Text {
        title: String,
        body: String,
        scroll: u16,
    },
    Pending {
        request: Value,
        generation: u64,
        thread: String,
        answers: BTreeMap<String, Value>,
        question: usize,
        input: String,
        scroll: u16,
    },
    Move {
        selected: usize,
        edge: usize,
    },
    Help,
}
#[derive(Clone)]
enum Hit {
    Find,
    Tasks,
    ToggleSide,
    Result(usize),
    Task(String),
    Live(String),
    Model(usize),
    Open(String),
    Tab(String),
    New,
    Search,
    Help,
    Pending,
    Sidebar,
    Group(String),
    ClearFilter,
    ArchiveFilter,
    Side,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum ManagerTarget {
    Directory(String),
    Conversation(String),
}
struct ManagerRow {
    title: String,
    target: Option<ManagerTarget>,
}
struct ModelChoice {
    value: String,
    name: String,
    description: String,
    default: bool,
}
fn model_value(model: &Value) -> String {
    model["model"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| model["id"].as_str())
        .unwrap_or_default()
        .trim()
        .into()
}
fn directory(c: &Conversation) -> String {
    let path = c.cwd.trim_end_matches('/');
    if path.is_empty() {
        if c.cwd.starts_with('/') {
            std::path::MAIN_SEPARATOR.to_string()
        } else {
            "No directory".into()
        }
    } else {
        path.into()
    }
}
fn path_tail(path: &str, width: usize) -> String {
    if path.width() <= width {
        return path.into();
    }
    let mut tail = String::new();
    let mut used = 1;
    for g in path.graphemes(true).rev() {
        if used + g.width() > width {
            break;
        }
        tail.insert_str(0, g);
        used += g.width();
    }
    format!("…{tail}")
}
#[derive(Clone)]
enum Drag {
    Conversation { key: String, x: u16, y: u16 },
    Divider(Divider),
    Sidebar,
}
#[derive(Clone)]
enum Effect {
    SearchContext,
    Model { thread: String, model: String },
    Open,
    Side { parent: String, prompt: String },
    Send { thread: String, text: String },
    Diff { thread: String },
    Login,
    Generic,
}

struct CachedLine {
    line: Line<'static>,
    start: usize,
    height: usize,
}
#[derive(Default)]
struct CachedLines {
    lines: Vec<CachedLine>,
    height: usize,
}
impl CachedLines {
    fn new(lines: impl IntoIterator<Item = Line<'static>>, width: u16) -> Self {
        let mut cache = Self::default();
        for line in lines {
            let height = Paragraph::new(line.clone())
                .wrap(Wrap { trim: false })
                .line_count(width);
            cache.lines.push(CachedLine {
                line,
                start: cache.height,
                height,
            });
            cache.height += height;
        }
        cache
    }
    fn render(&self, f: &mut Frame, area: Rect, skip: &mut usize, used: &mut u16) {
        if *skip >= self.height {
            *skip -= self.height;
            return;
        }
        let first = self
            .lines
            .partition_point(|line| line.start + line.height <= *skip);
        for line in &self.lines[first..] {
            if *used >= area.height {
                break;
            }
            let offset = skip.saturating_sub(line.start);
            let height = (line.height - offset).min((area.height - *used) as usize) as u16;
            f.render_widget(
                Paragraph::new(line.line.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((offset.min(u16::MAX as usize) as u16, 0)),
                Rect::new(area.x, area.y + *used, area.width, height),
            );
            *used += height;
        }
        *skip = 0;
    }
}
struct CachedMessage {
    source: Message,
    rendered: CachedLines,
}
#[derive(Default)]
struct Transcript {
    width: u16,
    messages: Vec<CachedMessage>,
    prefix: CachedLines,
    suffix: CachedLines,
    height: usize,
}
impl Transcript {
    fn update(&mut self, c: &Conversation, parent: Option<&str>, width: u16) {
        if self.width != width {
            self.messages.clear();
            self.width = width;
        }
        self.prefix = CachedLines::new(
            parent.into_iter().flat_map(|parent| {
                [
                    Line::styled(
                        format!("Side of {parent} · /send-parent to return a draft"),
                        style(ACCENT),
                    ),
                    Line::default(),
                ]
            }),
            width,
        );
        for (i, message) in c.messages.iter().enumerate() {
            if self
                .messages
                .get(i)
                .is_some_and(|cached| cached.source == *message)
            {
                continue;
            }
            let rc = match message.role.as_str() {
                "You" => ACCENT,
                "Codex" => FG,
                "Error" => Color::LightRed,
                _ => MUTED,
            };
            let status = match message.status.as_str() {
                "inProgress" | "running" => " · ● Running",
                "completed" => " · ✓ Completed",
                "failed" => " · ✗ Failed",
                "declined" => " · ✗ Declined",
                "interrupted" => " · ■ Interrupted",
                "unknown" => " · ? Unknown",
                _ => "",
            };
            let color = if matches!(message.status.as_str(), "failed" | "declined") {
                Color::LightRed
            } else {
                rc
            };
            let mut lines = vec![Line::styled(
                format!("{}{status}", message.role),
                style(color).add_modifier(Modifier::BOLD),
            )];
            let body_style = style(if message.role == "Command" || message.role == "Thinking" {
                MUTED
            } else {
                FG
            });
            if message.role == "Codex" || message.role == "Thinking" {
                lines.extend(crate::markup::markdown(&message.text, body_style));
            } else {
                lines.extend(
                    crate::markup::clean(&message.text)
                        .lines()
                        .map(|line| Line::styled(line.to_owned(), body_style)),
                );
            }
            if message.role == "Thinking" && message.text.trim().is_empty() {
                lines.push(Line::styled(
                    if message.status == "inProgress" {
                        "Waiting for a reasoning summary…"
                    } else {
                        "No reasoning summary was provided."
                    },
                    style(MUTED),
                ));
            }
            lines.push(Line::default());
            let cached = CachedMessage {
                source: message.clone(),
                rendered: CachedLines::new(lines, width),
            };
            if i < self.messages.len() {
                self.messages[i] = cached;
            } else {
                self.messages.push(cached);
            }
        }
        self.messages.truncate(c.messages.len());
        let mut suffix = vec![];
        if c.messages.is_empty() {
            suffix.push(Line::styled(
                "Ask Codex to work in this directory.",
                style(MUTED),
            ));
            suffix.push(Line::styled(
                "/side <question> or /btw <question> opens a side conversation.",
                style(MUTED),
            ));
        }
        if c.busy() && !c.messages.iter().any(|m| m.status == "inProgress") {
            suffix.push(Line::styled(
                "● Waiting for the next Codex update…",
                style(MUTED),
            ));
        }
        for q in &c.queued {
            suffix.push(Line::styled(
                format!("◷ {} · {}", q.state, q.text),
                style(Color::Yellow),
            ));
        }
        self.suffix = CachedLines::new(suffix, width);
        self.height = self.prefix.height
            + self
                .messages
                .iter()
                .map(|m| m.rendered.height)
                .sum::<usize>()
            + self.suffix.height;
    }
    fn render(&self, f: &mut Frame, area: Rect, mut skip: usize) {
        let mut used = 0;
        for block in std::iter::once(&self.prefix)
            .chain(self.messages.iter().map(|m| &m.rendered))
            .chain(std::iter::once(&self.suffix))
        {
            if used >= area.height {
                break;
            }
            block.render(f, area, &mut skip, &mut used);
        }
    }
}

pub struct App {
    client: Option<Client>,
    snap: Snapshot,
    root: String,
    focus: String,
    layout: Node,
    layouts: BTreeMap<String, Node>,
    drafts: BTreeMap<String, Draft>,
    scroll: BTreeMap<String, u16>,
    heights: BTreeMap<String, (usize, String)>,
    transcripts: BTreeMap<String, Transcript>,
    dirty: BTreeSet<String>,
    modal: Option<Modal>,
    prefix: bool,
    resize: bool,
    sidebar: bool,
    side_width: u16,
    side_scroll: usize,
    sidebar_rect: Option<Rect>,
    manager_focused: bool,
    manager_filtering: bool,
    manager_query: String,
    manager_archived: bool,
    manager_selected: Option<ManagerTarget>,
    manager_reveal: bool,
    collapsed_dirs: BTreeSet<String>,
    collapsed_side_panels: BTreeSet<String>,
    search_views: BTreeMap<String, Conversation>,
    jump_targets: BTreeMap<String, String>,
    filter_collapsed: BTreeSet<String>,
    hits: Vec<(Rect, Hit)>,
    panes: Vec<Pane>,
    dividers: Vec<Divider>,
    drag: Option<Drag>,
    effects: HashMap<u64, Effect>,
    undo: Vec<Node>,
    maximize: Option<Node>,
    toast: String,
    toast_at: Instant,
    last_save: Instant,
    last_read: Instant,
    quit: bool,
    booted: bool,
    demo: bool,
}
impl App {
    pub fn new(client: Option<Client>) -> Self {
        Self {
            client,
            snap: Snapshot::default(),
            root: String::new(),
            focus: String::new(),
            layout: Node::single(""),
            layouts: BTreeMap::new(),
            drafts: BTreeMap::new(),
            scroll: BTreeMap::new(),
            heights: BTreeMap::new(),
            transcripts: BTreeMap::new(),
            dirty: BTreeSet::new(),
            modal: None,
            prefix: false,
            resize: false,
            sidebar: true,
            side_width: 28,
            side_scroll: 0,
            sidebar_rect: None,
            manager_focused: false,
            manager_filtering: false,
            manager_query: String::new(),
            manager_archived: false,
            manager_selected: None,
            manager_reveal: false,
            collapsed_dirs: BTreeSet::new(),
            collapsed_side_panels: BTreeSet::new(),
            search_views: BTreeMap::new(),
            jump_targets: BTreeMap::new(),
            filter_collapsed: BTreeSet::new(),
            hits: vec![],
            panes: vec![],
            dividers: vec![],
            drag: None,
            effects: HashMap::new(),
            undo: vec![],
            maximize: None,
            toast: String::new(),
            toast_at: Instant::now(),
            last_save: Instant::now(),
            last_read: Instant::now(),
            quit: false,
            booted: false,
            demo: false,
        }
    }
    fn request(&mut self, op: &str, args: Value, effect: Effect) {
        if let Some(client) = &mut self.client {
            let id = client.send(op, args);
            self.effects.insert(id, effect);
        } else {
            self.notice("Demo mode · backend actions are disabled");
        }
    }
    fn fire(&mut self, op: &str, args: Value) {
        if let Some(c) = &mut self.client {
            c.send(op, args);
        }
    }
    fn notice(&mut self, s: impl Into<String>) {
        self.toast = s.into();
        self.toast_at = Instant::now();
    }
    fn draft(&mut self) -> &mut Draft {
        self.drafts.entry(self.focus.clone()).or_default()
    }
    fn persist(&mut self) {
        if self.root.is_empty() {
            return;
        }
        let saved = self.maximize.as_ref().unwrap_or(&self.layout).clone();
        self.layouts.insert(self.root.clone(), saved.clone());
        self.fire(
            "view",
            json!({"root":self.root,"layout":saved,"sidebar":self.sidebar,"width":self.side_width,"collapsedDirectories":self.collapsed_dirs,"collapsedSidePanels":self.collapsed_side_panels}),
        );
    }
    fn remember(&mut self) {
        if let Some(old) = self.maximize.take() {
            self.layout = old;
        }
        self.undo.push(self.layout.clone());
        if self.undo.len() > 20 {
            self.undo.remove(0);
        }
    }
    fn focus(&mut self, key: &str) {
        if key != self.root {
            self.collapsed_side_panels.remove(&self.root);
        }
        self.manager_focused = false;
        self.manager_filtering = false;
        self.focus = key.into();
        self.layout.focus(key);
        self.fire("read", json!({"thread":key}));
    }
    fn workspace_root(&self, key: &str) -> String {
        let mut root = key.to_owned();
        let mut seen = BTreeSet::new();
        while seen.insert(root.clone()) {
            let Some(parent) = self
                .snap
                .data
                .conversations
                .get(&root)
                .and_then(|c| c.parent.as_ref())
            else {
                break;
            };
            if !self.snap.data.conversations.contains_key(parent) || seen.contains(parent) {
                break;
            }
            root = parent.clone();
        }
        root
    }
    fn switch_workspace(&mut self, root: &str) {
        self.persist();
        self.root = root.into();
        if !self.layouts.contains_key(root) {
            self.collapsed_side_panels.insert(root.into());
        }
        self.layout = self
            .layouts
            .get(root)
            .cloned()
            .unwrap_or_else(|| Node::single(root));
        self.maximize = None;
        self.undo.clear();
        let mut ids = vec![];
        self.layout.ids(&mut ids);
        if !ids.iter().any(|id| id == root) {
            if let Some(target) = ids.first() {
                self.layout.dock(root, target, "left");
            } else {
                self.layout = Node::single(root);
            }
        }
    }
    fn show_side(&mut self, key: &str, parent: &str) {
        let root = self.workspace_root(parent);
        if self.root != root {
            self.switch_workspace(&root);
        }
        let mut visible = vec![];
        self.layout.ids(&mut visible);
        if !visible.contains(&root) {
            if let Some(target) = visible.first() {
                self.layout.dock(&root, target, "left");
            } else {
                self.layout = Node::single(&root);
            }
            visible.clear();
            self.layout.ids(&mut visible);
        }
        if !visible.iter().any(|id| id == key) {
            self.remember();
            let existing = visible
                .iter()
                .find(|id| **id != root && self.workspace_root(id) == root)
                .cloned();
            let edge = if existing.is_some() || self.layout.leaves() >= 4 {
                "center"
            } else {
                "right"
            };
            let target = existing.as_deref().unwrap_or(&root);
            if !self.layout.dock(key, target, edge) {
                self.layout = Node::single(&root);
                self.layout.dock(key, &root, "right");
            }
        }
        self.focus(key);
    }
    fn open(&mut self, key: &str) {
        let mut ids = vec![];
        self.layout.ids(&mut ids);
        if let Some(parent) = self
            .snap
            .data
            .conversations
            .get(key)
            .and_then(|c| c.parent.clone())
        {
            self.show_side(key, &parent);
        } else if ids.iter().any(|id| id == key) {
            self.focus(key);
        } else {
            self.switch_workspace(key);
            self.focus(key);
        }
        if let Some(c) = self.snap.data.conversations.get(key) {
            self.collapsed_dirs.remove(&directory(c));
        }
        self.manager_selected = Some(ManagerTarget::Conversation(key.into()));
        self.manager_reveal = true;
        let mut visible = vec![];
        self.layout.active_ids(&mut visible);
        for id in visible {
            if id == key
                || self
                    .snap
                    .data
                    .conversations
                    .get(&id)
                    .is_some_and(|c| !c.loaded && !id.starts_with("local-"))
            {
                self.request("open", json!({"thread":id}), Effect::Generic);
            }
        }
        self.persist();
    }
    fn receive(&mut self) -> bool {
        let mut values = vec![];
        if let Some(c) = &self.client {
            for _ in 0..32 {
                let Ok(v) = c.rx.try_recv() else {
                    break;
                };
                if v["type"] == "snapshot"
                    && values
                        .last()
                        .is_some_and(|last: &Value| last["type"] == "snapshot")
                {
                    values.pop();
                }
                values.push(v);
            }
        }
        let changed = !values.is_empty();
        for mut v in values {
            match v["type"].as_str() {
                Some("snapshot" | "delta") => {
                    let snapshot = if v["type"] == "delta" {
                        serde_json::from_value::<SnapshotDelta>(v["delta"].take())
                            .map(|delta| delta.apply(&mut self.snap))
                    } else {
                        serde_json::from_value::<Snapshot>(v["snapshot"].take())
                    };
                    if let Ok(snapshot) = snapshot {
                        for (old, new) in &snapshot.data.aliases {
                            if self.collapsed_side_panels.remove(old) {
                                self.collapsed_side_panels.insert(new.clone());
                            }
                            self.layout.replace(old, new);
                            if let Some(Modal::Models { thread, .. }) = &mut self.modal {
                                if thread == old {
                                    *thread = new.clone();
                                }
                            }
                            if self.manager_selected
                                == Some(ManagerTarget::Conversation(old.clone()))
                            {
                                self.manager_selected =
                                    Some(ManagerTarget::Conversation(new.clone()));
                            }
                            if self.root == *old {
                                self.root = new.clone()
                            }
                            if self.focus == *old {
                                self.focus = new.clone()
                            }
                            if let Some(d) = self.drafts.remove(old) {
                                self.drafts.insert(new.clone(), d);
                            }
                        }
                        for (k, d) in &snapshot.data.drafts {
                            self.drafts.entry(k.clone()).or_insert_with(|| d.clone());
                        }
                        if !self.booted {
                            self.layouts = snapshot.data.layouts.clone();
                            self.scroll = snapshot.data.scroll.clone();
                            self.sidebar = snapshot.data.sidebar;
                            self.side_width = snapshot.data.sidebar_width.clamp(18, 52);
                            self.collapsed_dirs =
                                snapshot.data.collapsed_dirs.iter().cloned().collect();
                            self.collapsed_side_panels = snapshot
                                .data
                                .collapsed_side_panels
                                .iter()
                                .cloned()
                                .collect();
                            self.collapsed_side_panels.extend(
                                snapshot
                                    .data
                                    .conversations
                                    .values()
                                    .filter(|c| c.parent.is_none() || c.promoted)
                                    .map(|c| c.id.clone()),
                            );
                        }
                        self.snap = snapshot;
                        if !self.booted && self.snap.connected {
                            let key = self
                                .snap
                                .data
                                .active
                                .clone()
                                .filter(|k| self.snap.data.conversations.contains_key(k))
                                .or_else(|| self.sorted(false, "").first().cloned());
                            if let Some(k) = key {
                                self.booted = true;
                                self.open(&k);
                            }
                        }
                    }
                }
                Some("response") => {
                    let effect = self.effects.remove(&v["id"].as_u64().unwrap_or(0));
                    if let Some(e) = v["error"].as_str() {
                        self.notice(e);
                        continue;
                    }
                    let r = &v["result"];
                    match effect {
                        Some(Effect::SearchContext) => {
                            if let Ok(context) = serde_json::from_value::<SearchContext>(r.clone())
                            {
                                let hit = context.hit;
                                let c = self
                                    .snap
                                    .data
                                    .conversations
                                    .entry(hit.thread.clone())
                                    .or_insert_with(|| {
                                        let mut c = Conversation::new(
                                            hit.thread.clone(),
                                            hit.title.clone(),
                                            hit.cwd.clone(),
                                        );
                                        c.archived = hit.archived;
                                        c
                                    });
                                let mut view = c.clone();
                                view.messages = context.messages;
                                self.search_views.insert(hit.thread.clone(), view);
                                self.jump_targets.insert(hit.thread.clone(), hit.message);
                                self.heights.remove(&hit.thread);
                                self.open(&hit.thread);
                                self.notice(
                                    "Indexed search context · /live returns to latest messages",
                                );
                            }
                        }
                        Some(Effect::Model { thread, model }) => {
                            if let Some(c) = self.snap.data.conversations.get_mut(&thread) {
                                c.model = model.clone();
                            }
                            self.notice(format!("Model set to {model} · applies to the next task"));
                        }
                        Some(Effect::Open) => {
                            let k = text(r, "thread");
                            if !k.is_empty() {
                                self.open(&k)
                            }
                        }
                        Some(Effect::Side { parent, prompt }) => {
                            let k = text(r, "thread");
                            if !k.is_empty() {
                                self.show_side(&k, &parent);
                                self.persist();
                                if !prompt.is_empty() {
                                    self.request(
                                        "send",
                                        json!({"thread":k,"text":prompt}),
                                        Effect::Send {
                                            thread: k.clone(),
                                            text: prompt,
                                        },
                                    );
                                }
                            }
                        }
                        Some(Effect::Send { thread, text: sent }) => {
                            let k = text(r, "thread");
                            if !k.is_empty() && k != thread {
                                self.layout.replace(&thread, &k);
                                if self.root == thread {
                                    self.root = k.clone();
                                }
                                if self.focus == thread {
                                    self.focus = k.clone();
                                }
                                if let Some(d) = self.drafts.remove(&thread) {
                                    self.drafts.insert(k.clone(), d);
                                }
                                self.persist();
                            }
                            let target = if k.is_empty() { thread } else { k };
                            if let Some(d) = self.drafts.get_mut(&target) {
                                if d.text == sent {
                                    d.text.clear();
                                    d.cursor = 0;
                                }
                            }
                            self.dirty.insert(target);
                        }
                        Some(Effect::Diff { thread }) => {
                            self.modal = Some(Modal::Text {
                                title: text(r, "label"),
                                body: format!(
                                    "Turn diff (from Codex):\n{}\n\nWorkspace diff:\n{}",
                                    self.snap
                                        .data
                                        .conversations
                                        .get(&thread)
                                        .map(|c| c.diff.as_str())
                                        .unwrap_or(""),
                                    text(r, "diff")
                                ),
                                scroll: 0,
                            })
                        }
                        Some(Effect::Login) => {
                            self.modal = Some(Modal::Text {
                                title: "Codex login · open the verification URL".into(),
                                body: serde_json::to_string_pretty(r).unwrap_or_default(),
                                scroll: 0,
                            })
                        }
                        _ => {
                            if let Some(m) = r["message"].as_str() {
                                self.notice(m)
                            }
                        }
                    }
                }
                Some("disconnect") => {
                    self.notice("Background service disconnected. Exit and reopen Codex Deck to reconnect; drafts remain local.");
                    self.snap.connected = false;
                }
                _ => {}
            }
        }
        changed
    }
    fn sorted(&self, archived: bool, query: &str) -> Vec<String> {
        let query = query.to_lowercase();
        let mut c: Vec<_> = self
            .snap
            .data
            .conversations
            .values()
            .filter(|c| {
                c.archived == archived
                    && format!(
                        "{} {} {} {}",
                        c.title,
                        c.cwd,
                        c.project.as_deref().unwrap_or(""),
                        c.id
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect();
        c.sort_by(|a, b| {
            let pos = |key: &str| {
                self.snap
                    .data
                    .order
                    .iter()
                    .position(|k| k == key)
                    .unwrap_or(usize::MAX)
            };
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| pos(&a.id).cmp(&pos(&b.id)))
                .then(b.updated.cmp(&a.updated))
                .then(a.id.cmp(&b.id))
        });
        c.into_iter().map(|c| c.id.clone()).collect()
    }
    fn rows(&self) -> Vec<ManagerRow> {
        fn append(
            key: &str,
            depth: usize,
            keys: &[String],
            data: &Persisted,
            seen: &mut BTreeSet<String>,
            out: &mut Vec<ManagerRow>,
        ) {
            if !seen.insert(key.into()) {
                return;
            }
            let c = &data.conversations[key];
            out.push(ManagerRow {
                title: format!(
                    "{}{}{}",
                    "  ".repeat(depth.min(3)),
                    if depth > 0 || c.parent.is_some() {
                        "↳ "
                    } else {
                        ""
                    },
                    label(c)
                ),
                target: Some(ManagerTarget::Conversation(key.into())),
            });
            for child in keys
                .iter()
                .filter(|id| data.conversations[*id].parent.as_deref() == Some(key))
            {
                append(child, depth + 1, keys, data, seen, out);
            }
        }
        let sorted = self.sorted(self.manager_archived, "");
        let matching = self.sorted(self.manager_archived, &self.manager_query);
        let mut visible: BTreeSet<String> = matching.iter().cloned().collect();
        for key in matching {
            let mut current = key.clone();
            let mut seen = BTreeSet::new();
            while seen.insert(current.clone()) {
                let Some(parent) = self.snap.data.conversations[&current].parent.as_ref() else {
                    break;
                };
                let Some(c) = self.snap.data.conversations.get(parent) else {
                    break;
                };
                if c.archived != self.manager_archived
                    || directory(c) != directory(&self.snap.data.conversations[&key])
                {
                    break;
                }
                visible.insert(parent.clone());
                current = parent.clone();
            }
        }
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for key in sorted.into_iter().filter(|key| visible.contains(key)) {
            groups
                .entry(directory(&self.snap.data.conversations[&key]))
                .or_default()
                .push(key);
        }
        let mut out = vec![];
        for (cwd, keys) in groups {
            let collapsed = if self.manager_query.is_empty() {
                self.collapsed_dirs.contains(&cwd)
            } else {
                self.filter_collapsed.contains(&cwd)
            };
            let name = std::path::Path::new(&cwd)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&cwd);
            out.push(ManagerRow {
                title: format!(
                    "{} {} ({})",
                    if collapsed { "▸" } else { "▾" },
                    name,
                    keys.len()
                ),
                target: Some(ManagerTarget::Directory(cwd.clone())),
            });
            out.push(ManagerRow {
                title: cwd,
                target: None,
            });
            if collapsed {
                continue;
            }
            let mut seen = BTreeSet::new();
            for key in &keys {
                if self.snap.data.conversations[key]
                    .parent
                    .as_ref()
                    .is_some_and(|p| keys.contains(p))
                {
                    continue;
                }
                append(key, 0, &keys, &self.snap.data, &mut seen, &mut out);
            }
            for key in &keys {
                if !seen.contains(key) {
                    append(key, 0, &keys, &self.snap.data, &mut seen, &mut out);
                }
            }
        }
        out
    }
    fn focus_manager(&mut self, filter: bool) {
        self.sidebar = true;
        self.manager_focused = true;
        self.manager_filtering = filter;
        if filter {
            self.manager_query.clear();
            self.filter_collapsed.clear();
        }
        self.manager_selected = Some(ManagerTarget::Conversation(self.focus.clone()));
        self.manager_reveal = true;
    }
    fn manager_query_changed(&mut self) {
        self.side_scroll = 0;
        self.filter_collapsed.clear();
        self.manager_selected = self
            .sorted(self.manager_archived, &self.manager_query)
            .first()
            .cloned()
            .map(ManagerTarget::Conversation);
        self.manager_reveal = true;
    }
    fn toggle_directory(&mut self, path: &str) {
        let collapsed = if self.manager_query.is_empty() {
            &mut self.collapsed_dirs
        } else {
            &mut self.filter_collapsed
        };
        if !collapsed.remove(path) {
            collapsed.insert(path.into());
        }
        self.manager_selected = Some(ManagerTarget::Directory(path.into()));
        self.manager_reveal = true;
        self.persist();
    }
    fn manager_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let rows = self.rows();
        let targets: Vec<_> = rows.iter().filter_map(|row| row.target.clone()).collect();
        let selected = targets
            .iter()
            .position(|target| Some(target) == self.manager_selected.as_ref())
            .unwrap_or(0);
        match k.code {
            KeyCode::Char('d') if ctrl => self.quit = true,
            KeyCode::F(1) => self.modal = Some(Modal::Help),
            KeyCode::Tab | KeyCode::BackTab => {
                let key = self.focus.clone();
                self.focus(&key);
            }
            KeyCode::Esc => {
                if self.manager_query.is_empty() {
                    let key = self.focus.clone();
                    self.focus(&key);
                } else {
                    self.manager_query.clear();
                    self.manager_query_changed();
                }
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let step = if matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
                    8
                } else {
                    1
                };
                let index = if matches!(k.code, KeyCode::Up | KeyCode::PageUp) {
                    selected.saturating_sub(step)
                } else {
                    (selected + step).min(targets.len().saturating_sub(1))
                };
                self.manager_selected = targets.get(index).cloned();
                self.manager_reveal = true;
            }
            KeyCode::Enter => match self.manager_selected.clone() {
                Some(ManagerTarget::Conversation(key)) => self.open(&key),
                Some(ManagerTarget::Directory(path)) => self.toggle_directory(&path),
                None => {}
            },
            KeyCode::Left | KeyCode::Right if !self.manager_filtering => {
                if let Some(ManagerTarget::Directory(path)) = self.manager_selected.clone() {
                    let collapsed = if self.manager_query.is_empty() {
                        &mut self.collapsed_dirs
                    } else {
                        &mut self.filter_collapsed
                    };
                    if k.code == KeyCode::Left {
                        collapsed.insert(path);
                    } else {
                        collapsed.remove(&path);
                    }
                    self.persist();
                } else if k.code == KeyCode::Right {
                    let key = self.focus.clone();
                    self.focus(&key);
                }
            }
            KeyCode::Char('/') if !self.manager_filtering => self.manager_filtering = true,
            KeyCode::Backspace => {
                pop(&mut self.manager_query);
                self.manager_filtering = true;
                self.manager_query_changed();
            }
            KeyCode::Char('u') if ctrl => {
                self.manager_query.clear();
                self.manager_query_changed();
            }
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.manager_filtering = true;
                self.manager_query.push(c);
                self.manager_query_changed();
            }
            _ => {}
        }
    }
    fn save_drafts(&mut self) {
        let keys: Vec<_> = self.dirty.iter().cloned().collect();
        self.dirty.clear();
        for k in keys {
            if let Some(d) = self.drafts.get(&k) {
                self.fire("draft", json!({"thread":k,"draft":d}));
            }
        }
        self.last_save = Instant::now();
    }
    fn insert(&mut self, s: &str) {
        if self.focus.is_empty() {
            return;
        }
        let d = self.draft();
        d.cursor = d.cursor.min(d.text.len());
        while !d.text.is_char_boundary(d.cursor) {
            d.cursor = d.cursor.saturating_sub(1)
        }
        d.text.insert_str(d.cursor, s);
        d.cursor += s.len();
        self.dirty.insert(self.focus.clone());
    }
    fn submit(&mut self) {
        let key = self.focus.clone();
        if key.is_empty() {
            self.new_form();
            return;
        }
        let value = self.draft().text.clone();
        if self
            .effects
            .values()
            .any(|e| matches!(e,Effect::Send{thread,text} if thread==&key&&text==&value))
        {
            self.notice("Sending this message…");
            return;
        }
        if value.trim().is_empty() {
            return;
        }
        if value.starts_with('/') && !value.starts_with("//") {
            self.command(&value);
            return;
        }
        let value = if value.starts_with("//") {
            value[1..].into()
        } else {
            value
        };
        self.live(&key);
        self.request(
            "send",
            json!({"thread":key,"text":value}),
            Effect::Send {
                thread: key,
                text: value,
            },
        );
    }
    fn new_form(&mut self) {
        let cwd = self
            .snap
            .data
            .conversations
            .get(&self.focus)
            .map(|c| c.cwd.clone())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        self.modal = Some(Modal::New {
            fields: vec![cwd, String::new(), String::new()],
            focus: 0,
            readonly: false,
        });
    }
    fn search(&mut self, archived: bool) {
        self.manager_archived = archived;
        self.focus_manager(true);
        if archived {
            self.fire("refresh", json!({"archived":true}));
        }
    }
    fn quick_switch(&mut self) {
        self.modal = Some(Modal::Search {
            query: String::new(),
            selected: 0,
            archived: false,
        });
    }
    fn side(&mut self, prompt: String) {
        if self.focus.is_empty() {
            self.notice("Choose a conversation first");
            return;
        }
        if self.focus.starts_with("local-") {
            self.notice("Send the first message before opening a side conversation");
            return;
        }
        let parent = self.focus.clone();
        self.request(
            "side",
            json!({"thread":parent}),
            Effect::Side { parent, prompt },
        );
    }
    fn pending(&mut self) {
        let item = self
            .snap
            .pending
            .iter()
            .find(|r| r.thread == self.focus)
            .or_else(|| self.snap.pending.first())
            .cloned();
        if let Some(p) = item {
            self.open(&p.thread);
            self.modal = Some(Modal::Pending {
                request: p.id,
                generation: p.generation,
                thread: p.thread,
                answers: BTreeMap::new(),
                question: 0,
                input: String::new(),
                scroll: 0,
            });
        } else {
            self.notice("No pending approvals or questions");
        }
    }
    fn prompt(&mut self, title: &str, action: &str, value: &str) {
        self.modal = Some(Modal::Prompt {
            title: title.into(),
            value: value.into(),
            action: action.into(),
        });
    }
    fn side_visible(&self) -> bool {
        let mut ids = vec![];
        self.layout.ids(&mut ids);
        !self.collapsed_side_panels.contains(&self.root) && ids.iter().any(|id| id != &self.root)
    }
    fn toggle_side(&mut self, show: Option<bool>) {
        let show = show.unwrap_or_else(|| !self.side_visible());
        if show {
            let mut ids = vec![];
            self.layout.ids(&mut ids);
            if !ids.iter().any(|id| id != &self.root) {
                self.notice("Use /side or /btw to open a side conversation");
                return;
            }
            self.collapsed_side_panels.remove(&self.root);
        } else {
            if let Some(layout) = self.maximize.take() {
                self.layout = layout;
            }
            self.collapsed_side_panels.insert(self.root.clone());
            let root = self.root.clone();
            self.focus(&root);
        }
        self.persist();
    }
    fn live(&mut self, key: &str) {
        self.search_views.remove(key);
        self.jump_targets.remove(key);
        self.heights.remove(key);
        self.scroll.insert(key.into(), 0);
    }
    fn find(&mut self, query: &str) {
        self.modal = Some(Modal::Find {
            query: query.into(),
            submitted: query.into(),
            selected: 0,
        });
        if !query.trim().is_empty() {
            self.request("search", json!({"query":query}), Effect::Generic);
        }
    }
    fn activate_result(&mut self, index: usize) {
        if let Some(hit) = self.snap.search.hits.get(index).cloned() {
            self.modal = None;
            self.fire("search_cancel", json!({"job":self.snap.search.job}));
            self.request(
                "search_context",
                json!({"thread":hit.thread,"message":hit.message}),
                Effect::SearchContext,
            );
        }
    }
    fn task_kind(&self, c: &Conversation) -> usize {
        if c.status.starts_with("waiting") || self.snap.pending.iter().any(|p| p.thread == c.id) {
            3
        } else if matches!(c.status.as_str(), "failed" | "error" | "unknown") {
            4
        } else if c.busy() {
            1
        } else if !c.queued.is_empty() {
            2
        } else {
            0
        }
    }
    fn task_rows(&self, query: &str, filter: usize) -> Vec<String> {
        let query = query.to_lowercase();
        let mut rows: Vec<_> = self
            .snap
            .data
            .conversations
            .values()
            .filter(|c| {
                let kind = self.task_kind(c);
                kind > 0
                    && (filter == 0 || kind == filter)
                    && format!("{} {} {} {}", c.title, c.cwd, c.id, c.status)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect();
        rows.sort_by(|a, b| {
            let rank = |c: &Conversation| match self.task_kind(c) {
                3 => 0,
                4 => 1,
                1 => 2,
                _ => 3,
            };
            rank(a)
                .cmp(&rank(b))
                .then(b.updated.cmp(&a.updated))
                .then(a.id.cmp(&b.id))
        });
        rows.into_iter().map(|c| c.id.clone()).collect()
    }
    fn model_choices(&self, query: &str) -> Vec<ModelChoice> {
        let query = query.to_lowercase();
        let mut seen = BTreeSet::new();
        self.snap
            .models
            .iter()
            .filter_map(|model| {
                let value = model_value(model);
                if model["hidden"] == true || value.is_empty() || !seen.insert(value.clone()) {
                    return None;
                }
                let name = text(model, "displayName");
                let description = text(model, "description");
                let searchable = format!("{value} {name} {description} {}", text(model, "id"));
                searchable
                    .to_lowercase()
                    .contains(&query)
                    .then_some(ModelChoice {
                        name: if name.is_empty() { value.clone() } else { name },
                        value,
                        description,
                        default: model["isDefault"] == true,
                    })
            })
            .collect()
    }
    fn model_picker(&mut self) {
        let Some(c) = self.snap.data.conversations.get(&self.focus) else {
            self.notice("Open or create a conversation before selecting its model");
            return;
        };
        let choices = self.model_choices("");
        let selected = choices
            .iter()
            .position(|m| m.value == c.model)
            .or_else(|| choices.iter().position(|m| c.model.is_empty() && m.default))
            .unwrap_or(0);
        self.modal = Some(Modal::Models {
            thread: self.focus.clone(),
            query: String::new(),
            selected,
        });
    }
    fn set_model(&mut self, thread: &str, model: &str) -> bool {
        let Some(c) = self.snap.data.conversations.get(thread) else {
            self.notice("Conversation is no longer available");
            return false;
        };
        if c.busy() || !c.queued.is_empty() {
            self.notice("Stop the task and clear queued messages before changing the model");
            return false;
        }
        let model = model.trim();
        if model.is_empty() {
            return false;
        }
        let model = self
            .snap
            .models
            .iter()
            .find(|m| text(m, "id") == model || model_value(m) == model)
            .map(model_value)
            .unwrap_or_else(|| model.into());
        self.request(
            "settings",
            json!({"thread":thread,"model":model}),
            Effect::Model {
                thread: thread.into(),
                model,
            },
        );
        true
    }
    fn command(&mut self, input: &str) {
        let (cmd, arg) = input
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((input.trim(), ""));
        let arg = arg.trim();
        let k = self.focus.clone();
        match cmd {
            "/side-panel" => match arg {
                "show" => self.toggle_side(Some(true)),
                "hide" => self.toggle_side(Some(false)),
                _ => self.toggle_side(None),
            },
            "/find" => self.find(arg),
            "/tasks" => {
                self.modal = Some(Modal::Tasks {
                    query: String::new(),
                    selected: 0,
                    filter: 0,
                })
            }
            "/live" => self.live(&k),
            "/side" | "/btw" => self.side(arg.into()),
            "/new" => {
                if arg.is_empty() {
                    self.new_form()
                } else {
                    self.request("prepare", json!({"cwd":arg}), Effect::Open)
                }
            }
            "/search" => self.search(false),
            "/quick-switch" => self.quick_switch(),
            "/archived" => self.search(true),
            "/pin" => self.request("pin", json!({"thread":k}), Effect::Generic),
            "/rename" => {
                if arg.is_empty() {
                    self.prompt("Rename conversation", "rename", "")
                } else {
                    self.request("rename", json!({"thread":k,"title":arg}), Effect::Generic)
                }
            }
            "/cwd" => {
                if arg.is_empty() {
                    self.prompt("Directory for the next task", "cwd", "")
                } else {
                    self.request("settings", json!({"thread":k,"cwd":arg}), Effect::Generic)
                }
            }
            "/model" => {
                if arg.is_empty() {
                    self.model_picker();
                } else {
                    self.set_model(&k, arg);
                }
            }
            "/models" => self.model_picker(),
            "/permissions" => match arg {
                "read-only" | "readonly" | "discussion" => self.request(
                    "settings",
                    json!({"thread":k,"readonly":true}),
                    Effect::Generic,
                ),
                "workspace-write" | "execute" => self.request(
                    "settings",
                    json!({"thread":k,"readonly":false}),
                    Effect::Generic,
                ),
                _ => self.notice("Use /permissions read-only or /permissions workspace-write"),
            },
            "/project" => {
                if arg.is_empty() {
                    self.prompt("Assign to project (creates it if needed)", "project", "")
                } else {
                    self.request("project",json!({"thread":k,"name":arg,"cwd":self.snap.data.conversations.get(&k).map(|c|c.cwd.clone())}),Effect::Generic)
                }
            }
            "/worktree" => {
                if let Some((path, base)) = arg.rsplit_once(' ') {
                    self.request(
                        "worktree",
                        json!({"thread":k,"path":path,"base":base}),
                        Effect::Generic,
                    )
                } else {
                    self.notice("Usage: /worktree <absolute-directory> existing-ref")
                }
            }
            "/archive" => self.prompt(
                "Archive this conversation? Type ARCHIVE to confirm. Backend may archive forks.",
                "archive",
                "",
            ),
            "/restore" => self.request("restore", json!({"thread":k}), Effect::Generic),
            "/queue" => match arg {
                "clear" => self.request("queue", json!({"thread":k,"clear":true}), Effect::Generic),
                "release" => self.prompt(
                    "Release held queue? Type RELEASE after checking uncertain tasks in history.",
                    "release",
                    "",
                ),
                _ => {
                    self.modal = Some(Modal::Text {
                        title: "Queue · /queue clear · /queue release".into(),
                        body: self
                            .snap
                            .data
                            .conversations
                            .get(&k)
                            .map(|c| {
                                c.queued
                                    .iter()
                                    .map(|q| format!("[{}] {}\n{}", q.state, q.cwd, q.text))
                                    .collect::<Vec<_>>()
                                    .join("\n\n")
                            })
                            .unwrap_or_default(),
                        scroll: 0,
                    })
                }
            },
            "/steer" => {
                if arg.is_empty() {
                    self.prompt("Explicitly steer the active task", "steer", "")
                } else {
                    self.request(
                        "send",
                        json!({"thread":k,"text":arg,"steer":true}),
                        Effect::Generic,
                    )
                }
            }
            "/send-parent" => {
                if let Some(c) = self.snap.data.conversations.get(&k) {
                    if let Some(parent) = &c.parent {
                        let parent = parent.clone();
                        let summary = if arg.is_empty() {
                            c.messages
                                .iter()
                                .rev()
                                .find(|m| m.role == "Codex")
                                .map(|m| m.text.clone())
                                .unwrap_or_default()
                        } else {
                            arg.into()
                        };
                        self.open(&parent);
                        let d = self.drafts.entry(parent.clone()).or_default();
                        if !d.text.is_empty() {
                            d.text.push_str("\n\n")
                        }
                        d.text
                            .push_str(&format!("From side conversation:\n{summary}"));
                        d.cursor = d.text.len();
                        self.dirty.insert(parent);
                        self.notice(
                            "Side result copied to the parent draft. Edit and press Enter to send.",
                        );
                    }
                }
            }
            "/parent" => {
                if let Some(parent) = self
                    .snap
                    .data
                    .conversations
                    .get(&k)
                    .and_then(|c| c.parent.clone())
                {
                    self.open(&parent)
                }
            }
            "/promote" => self.request("promote", json!({"thread":k}), Effect::Generic),
            "/diff" => self.request(
                "diff",
                json!({"thread":k}),
                Effect::Diff { thread: k.clone() },
            ),
            "/history" => self.request("history", json!({"thread":k}), Effect::Generic),
            "/stop" => self.request("stop", json!({"thread":k}), Effect::Generic),
            "/reconnect" => self.request("reconnect", json!({}), Effect::Generic),
            "/refresh" => self.request("refresh", json!({}), Effect::Generic),
            "/login" => self.request("login", json!({}), Effect::Login),
            "/approvals" => self.pending(),
            "/move" => {
                self.modal = Some(Modal::Move {
                    selected: 0,
                    edge: 0,
                })
            }
            "/close" => self.close(),
            "/quit" => self.quit = true,
            "/help" => self.modal = Some(Modal::Help),
            _ => {
                self.notice("Unknown command · /help lists commands. Start with // to send a literal slash.");
                return;
            }
        }
        if let Some(d) = self.drafts.get_mut(&k) {
            if d.text == input {
                d.text.clear();
                d.cursor = 0;
                self.dirty.insert(k);
            }
        }
    }
    fn close(&mut self) {
        self.remember();
        let key = self.focus.clone();
        self.layout.remove(&key);
        let mut ids = vec![];
        self.layout.ids(&mut ids);
        if key != self.root && !ids.iter().any(|id| id != &self.root) {
            self.collapsed_side_panels.insert(self.root.clone());
        }
        self.focus = ids.first().cloned().unwrap_or_default();
        self.persist();
    }
    fn dock(&mut self, key: &str, target: &str, edge: &str) {
        if let Some(old) = self.maximize.take() {
            self.layout = old;
        }
        let old = self.layout.clone();
        if self.layout.dock(key, target, edge) {
            if self.layout.leaves() > 4 {
                self.layout = old;
                self.notice("Maximum four panes · drop in the center to create a tab");
                return;
            }
            self.undo.push(old);
            self.focus(key);
            self.request("open", json!({"thread":key}), Effect::Generic);
            self.persist();
        }
    }
    fn scroll(&mut self, delta: i32) {
        let k = self.focus.clone();
        let n = self.scroll.entry(k.clone()).or_default();
        *n = (*n as i32 + delta).clamp(0, 65535) as u16;
        let offset = *n;
        self.fire("scroll", json!({"thread":k,"offset":offset}));
    }
    fn direction(&mut self, dx: i32, dy: i32) {
        if self.manager_focused {
            if dx > 0 {
                let key = self.root.clone();
                self.focus(&key);
            }
            return;
        }
        let Some(current) = self
            .panes
            .iter()
            .find(|p| p.ids.iter().any(|id| id == &self.focus))
        else {
            return;
        };
        let cx = current.rect.x as i32 + current.rect.width as i32 / 2;
        let cy = current.rect.y as i32 + current.rect.height as i32 / 2;
        let target = self
            .panes
            .iter()
            .filter_map(|p| {
                let x = p.rect.x as i32 + p.rect.width as i32 / 2 - cx;
                let y = p.rect.y as i32 + p.rect.height as i32 / 2 - cy;
                if x * dx + y * dy <= 0 {
                    return None;
                }
                Some((x.abs() + y.abs() * 2, p.ids.get(p.active)?.clone()))
            })
            .min_by_key(|p| p.0)
            .map(|p| p.1);
        if let Some(k) = target {
            self.focus(&k);
            self.persist();
        } else if dx < 0 {
            self.focus_manager(false);
        }
    }
    fn tab(&mut self, delta: i32) {
        if let Some(p) = self.panes.iter().find(|p| p.ids.contains(&self.focus)) {
            if !p.ids.is_empty() {
                let i = p.ids.iter().position(|k| k == &self.focus).unwrap_or(0);
                let k = p.ids[(i as i32 + delta).rem_euclid(p.ids.len() as i32) as usize].clone();
                self.focus(&k);
                self.persist();
            }
        }
    }
    fn resize_key(&mut self, code: KeyCode) {
        let vertical = matches!(code, KeyCode::Up | KeyCode::Down);
        let path = self
            .panes
            .iter()
            .find(|p| p.ids.contains(&self.focus))
            .map(|p| p.path.clone())
            .unwrap_or_default();
        let d = self
            .dividers
            .iter()
            .rev()
            .find(|d| d.vertical == vertical && path.starts_with(&d.path))
            .cloned();
        if let Some(d) = d {
            self.remember();
            if let Some(Node::Split { ratio, .. }) = self.layout.at_mut(&d.path) {
                let plus = matches!(code, KeyCode::Right | KeyCode::Down);
                *ratio = if plus {
                    ratio.saturating_add(50)
                } else {
                    ratio.saturating_sub(50)
                }
                .clamp(150, 850);
            }
            self.persist();
        }
    }
    fn key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        if self.modal.is_some() {
            self.modal_key(k);
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('g') {
            self.prefix = !self.prefix;
            return;
        }
        if self.resize {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => self.resize = false,
                KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                    self.resize_key(k.code)
                }
                _ => {}
            }
            return;
        }
        if self.prefix {
            self.prefix = false;
            match k.code {
                KeyCode::Char('n') => self.new_form(),
                KeyCode::Char('p') => self.search(false),
                KeyCode::Char('/') => self.find(""),
                KeyCode::Char('t') => {
                    self.modal = Some(Modal::Tasks {
                        query: String::new(),
                        selected: 0,
                        filter: 0,
                    })
                }
                KeyCode::Char('o') => self.toggle_side(None),
                KeyCode::Char('f') => self.quick_switch(),
                KeyCode::Char('b') => {
                    self.sidebar = !self.sidebar;
                    if !self.sidebar {
                        self.manager_focused = false;
                    }
                    self.persist();
                }
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Char('s') => self.side(String::new()),
                KeyCode::Char('a') => self.pending(),
                KeyCode::Char('d') => self.request(
                    "diff",
                    json!({"thread":self.focus}),
                    Effect::Diff {
                        thread: self.focus.clone(),
                    },
                ),
                KeyCode::Char('m') => {
                    self.modal = Some(Modal::Move {
                        selected: 0,
                        edge: 0,
                    })
                }
                KeyCode::Char('r') => self.resize = true,
                KeyCode::Char('w') => self.close(),
                KeyCode::Char('z') => {
                    if let Some(old) = self.maximize.take() {
                        self.layout = old
                    } else {
                        self.maximize = Some(self.layout.clone());
                        self.layout = Node::single(&self.focus)
                    }
                }
                KeyCode::Char('u') => {
                    if let Some(n) = self.undo.pop() {
                        self.layout = n;
                        let mut ids = vec![];
                        self.layout.ids(&mut ids);
                        if ids.iter().any(|id| id != &self.root) {
                            self.collapsed_side_panels.remove(&self.root);
                        }
                        self.persist();
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => self.direction(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => self.direction(1, 0),
                KeyCode::Up | KeyCode::Char('k') => self.direction(0, -1),
                KeyCode::Down | KeyCode::Char('j') => self.direction(0, 1),
                KeyCode::Char('[') => self.tab(-1),
                KeyCode::Char(']') => self.tab(1),
                KeyCode::Char('?') => self.modal = Some(Modal::Help),
                KeyCode::Char(':') => {
                    self.prompt("Command · /help lists all commands", "command", "/")
                }
                _ => {}
            }
            return;
        }
        if self.manager_focused {
            self.manager_key(k);
            return;
        }
        match k.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Char('j') if ctrl => self.insert("\n"),
            KeyCode::Char('c') if ctrl => {
                if self
                    .snap
                    .data
                    .conversations
                    .get(&self.focus)
                    .is_some_and(Conversation::busy)
                {
                    self.request("stop", json!({"thread":self.focus}), Effect::Generic)
                } else {
                    self.notice("Ctrl+G then q detaches; tasks keep running");
                }
            }
            KeyCode::Char('d') if ctrl => self.quit = true,
            KeyCode::Backspace => {
                let d = self.draft();
                if d.cursor > 0 {
                    let i = d.text[..d.cursor.min(d.text.len())]
                        .grapheme_indices(true)
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    d.text.replace_range(i..d.cursor, "");
                    d.cursor = i;
                    self.dirty.insert(self.focus.clone());
                }
            }
            KeyCode::Delete => {
                let d = self.draft();
                if d.cursor < d.text.len() {
                    let n = d.text[d.cursor..]
                        .graphemes(true)
                        .next()
                        .map(str::len)
                        .unwrap_or(0);
                    d.text.replace_range(d.cursor..d.cursor + n, "");
                    self.dirty.insert(self.focus.clone());
                }
            }
            KeyCode::Left => {
                let d = self.draft();
                d.cursor = d.text[..d.cursor.min(d.text.len())]
                    .grapheme_indices(true)
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                let d = self.draft();
                if d.cursor < d.text.len() {
                    d.cursor += d.text[d.cursor..]
                        .graphemes(true)
                        .next()
                        .map(str::len)
                        .unwrap_or(0);
                }
            }
            KeyCode::Home => {
                self.draft().cursor = 0;
            }
            KeyCode::End => {
                let d = self.draft();
                d.cursor = d.text.len();
            }
            KeyCode::Up => vertical_cursor(self.draft(), false),
            KeyCode::Down => vertical_cursor(self.draft(), true),
            KeyCode::PageUp => self.scroll(12),
            KeyCode::PageDown => self.scroll(-12),
            KeyCode::Char('u') if ctrl => {
                self.draft().text.clear();
                self.draft().cursor = 0;
                self.dirty.insert(self.focus.clone());
            }
            KeyCode::Tab => self.tab(1),
            KeyCode::BackTab => self.tab(-1),
            KeyCode::F(1) => self.modal = Some(Modal::Help),
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(&c.to_string())
            }
            _ => {}
        }
    }
    fn modal_key(&mut self, k: KeyEvent) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        if k.code == KeyCode::Esc {
            if matches!(modal, Modal::Find { .. }) {
                self.fire("search_cancel", json!({"job":self.snap.search.job}));
            }
            return;
        }
        let mut keep = true;
        match &mut modal {
            Modal::Find {
                query,
                submitted,
                selected,
            } => {
                let current = *submitted == *query && self.snap.search.query == *query;
                let len = if current {
                    self.snap.search.hits.len()
                } else {
                    0
                };
                match k.code {
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(len.saturating_sub(1)),
                    KeyCode::PageUp => *selected = selected.saturating_sub(5),
                    KeyCode::PageDown => *selected = (*selected + 5).min(len.saturating_sub(1)),
                    KeyCode::Enter if current && len > 0 => {
                        self.activate_result((*selected).min(len - 1));
                        keep = false;
                    }
                    KeyCode::Enter | KeyCode::Char('r')
                        if k.code == KeyCode::Enter
                            || k.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if !query.trim().is_empty() {
                            *submitted = query.trim().into();
                            *query = submitted.clone();
                            *selected = 0;
                            self.request("search", json!({"query":query}), Effect::Generic);
                        }
                    }
                    KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.clear();
                        *selected = 0;
                    }
                    KeyCode::Backspace => {
                        pop(query);
                        *selected = 0;
                    }
                    KeyCode::Char(c)
                        if !k
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        query.push(c);
                        *selected = 0;
                    }
                    _ => {}
                }
            }
            Modal::Tasks {
                query,
                selected,
                filter,
            } => {
                let rows = self.task_rows(query, *filter);
                match k.code {
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(rows.len().saturating_sub(1)),
                    KeyCode::PageUp => *selected = selected.saturating_sub(5),
                    KeyCode::PageDown => {
                        *selected = (*selected + 5).min(rows.len().saturating_sub(1))
                    }
                    KeyCode::Tab => {
                        *filter = (*filter + 1) % 5;
                        *selected = 0;
                    }
                    KeyCode::BackTab => {
                        *filter = (*filter + 4) % 5;
                        *selected = 0;
                    }
                    KeyCode::Enter | KeyCode::F(2) => {
                        if let Some(key) = rows.get((*selected).min(rows.len().saturating_sub(1))) {
                            self.open(key);
                            if k.code == KeyCode::F(2)
                                && self.snap.pending.iter().any(|p| &p.thread == key)
                            {
                                self.pending();
                            }
                            keep = false;
                        }
                    }
                    KeyCode::Backspace => {
                        pop(query);
                        *selected = 0;
                    }
                    KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.clear();
                        *selected = 0;
                    }
                    KeyCode::Char(c)
                        if !k
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        query.push(c);
                        *selected = 0;
                    }
                    _ => {}
                }
            }
            Modal::Models {
                thread,
                query,
                selected,
            } => {
                let choices = self.model_choices(query);
                *selected = (*selected).min(choices.len().saturating_sub(1));
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Up | KeyCode::BackTab => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Tab => {
                        *selected = (*selected + 1).min(choices.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => *selected = selected.saturating_sub(5),
                    KeyCode::PageDown => {
                        *selected = (*selected + 5).min(choices.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(model) = choices.get(*selected) {
                            keep = !self.set_model(thread, &model.value);
                        }
                    }
                    KeyCode::Char('u') if ctrl => {
                        query.clear();
                        *selected = 0;
                    }
                    KeyCode::Backspace => {
                        pop(query);
                        *selected = 0;
                    }
                    KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                        query.push(c);
                        *selected = 0;
                    }
                    _ => {}
                }
            }
            Modal::Help => {
                if matches!(k.code, KeyCode::Enter | KeyCode::Char('q') | KeyCode::F(1)) {
                    keep = false
                }
            }
            Modal::Text { scroll, .. } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => *scroll = scroll.saturating_add(15),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(15),
                KeyCode::Char('q') => keep = false,
                _ => {}
            },
            Modal::Search {
                query,
                selected,
                archived,
            } => {
                let results = self.sorted(*archived, query);
                match k.code {
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => {
                        *selected = (*selected + 1).min(results.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(key) = results.get(*selected) {
                            self.open(key);
                            keep = false
                        }
                    }
                    KeyCode::Tab => {
                        *archived = !*archived;
                        *selected = 0;
                        if *archived {
                            self.fire("refresh", json!({"archived":true}));
                        }
                    }
                    KeyCode::Char(c) => {
                        query.push(c);
                        *selected = 0
                    }
                    KeyCode::Backspace => {
                        pop(query);
                        *selected = 0
                    }
                    _ => {}
                }
            }
            Modal::New {
                fields,
                focus,
                readonly,
            } => match k.code {
                KeyCode::Tab => *focus = (*focus + 1) % 3,
                KeyCode::BackTab => *focus = (*focus + 2) % 3,
                KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    *readonly = !*readonly
                }
                KeyCode::Enter => {
                    self.request("prepare",json!({"cwd":fields[0],"title":fields[1],"model":fields[2],"readonly":readonly}),Effect::Open);
                    keep = false
                }
                KeyCode::Char(c) => fields[*focus].push(c),
                KeyCode::Backspace => pop(&mut fields[*focus]),
                _ => {}
            },
            Modal::Prompt { value, action, .. } => match k.code {
                KeyCode::Enter => {
                    let k = self.focus.clone();
                    match action.as_str(){
                    "command"=>self.command(value),"rename"=>self.request("rename",json!({"thread":k,"title":value}),Effect::Generic),"cwd"=>self.request("settings",json!({"thread":k,"cwd":value}),Effect::Generic),"project"=>self.request("project",json!({"thread":k,"name":value,"cwd":self.snap.data.conversations.get(&k).map(|c|c.cwd.clone())}),Effect::Generic),
                    "steer"=>self.request("send",json!({"thread":k,"text":value,"steer":true}),Effect::Generic),
                    "archive"=>if value=="ARCHIVE"{self.request("archive",json!({"thread":k}),Effect::Generic)}else{self.notice("Archive cancelled")},
                    "release"=>if value=="RELEASE"{self.request("queue",json!({"thread":k}),Effect::Generic)}else{self.notice("Queue remains held")},_=>{}
                }
                    keep = false
                }
                KeyCode::Backspace => pop(value),
                KeyCode::Char(c) => value.push(c),
                _ => {}
            },
            Modal::Move { selected, edge } => {
                let mut ids = vec![];
                self.layout.ids(&mut ids);
                ids.retain(|k| k != &self.focus);
                match k.code {
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(ids.len().saturating_sub(1)),
                    KeyCode::Left => *edge = (*edge + 4) % 5,
                    KeyCode::Right | KeyCode::Tab => *edge = (*edge + 1) % 5,
                    KeyCode::Enter => {
                        if let Some(target) = ids.get(*selected) {
                            let source = self.focus.clone();
                            self.dock(
                                &source,
                                target,
                                ["right", "left", "up", "down", "center"][*edge],
                            );
                            keep = false
                        }
                    }
                    _ => {}
                }
            }
            Modal::Pending {
                request,
                answers,
                question,
                input,
                scroll,
                generation,
                thread,
            } => {
                if k.code == KeyCode::PageDown {
                    *scroll = scroll.saturating_add(10);
                    self.modal = Some(modal);
                    return;
                }
                if k.code == KeyCode::PageUp {
                    *scroll = scroll.saturating_sub(10);
                    self.modal = Some(modal);
                    return;
                }
                if let Some(p) = self
                    .snap
                    .pending
                    .iter()
                    .find(|p| {
                        &p.id == request && p.generation == *generation && &p.thread == thread
                    })
                    .cloned()
                {
                    let mut result = None;
                    if p.method.ends_with("requestUserInput") {
                        let questions = p.params["questions"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        match k.code {
                            KeyCode::Enter => {
                                if !input.trim().is_empty() {
                                    if let Some(q) = questions.get(*question) {
                                        let mut answer = input.clone();
                                        if let Ok(n) = input.parse::<usize>() {
                                            if let Some(option) = n.checked_sub(1).and_then(|i| {
                                                q["options"].as_array().and_then(|a| a.get(i))
                                            }) {
                                                answer = text(option, "label");
                                            }
                                        }
                                        answers.insert(text(q, "id"), json!({"answers":[answer]}));
                                        input.clear();
                                        *question += 1;
                                        if *question >= questions.len() {
                                            result = Some(json!({"answers":answers}));
                                        }
                                    }
                                }
                            }
                            KeyCode::Char(c) => input.push(c),
                            KeyCode::Backspace => pop(input),
                            _ => {}
                        }
                    } else if p.method.ends_with("requestApproval") {
                        match k.code {
                            KeyCode::Char('y') | KeyCode::Char('1') => {
                                result = Some(if p.method.contains("permissions") {
                                    json!({"permissions":p.params["permissions"],"scope":"turn"})
                                } else {
                                    json!({"decision":"accept"})
                                })
                            }
                            KeyCode::Char('n') | KeyCode::Char('2') => {
                                result = Some(if p.method.contains("permissions") {
                                    json!({"permissions":{},"scope":"turn"})
                                } else {
                                    json!({"decision":"decline"})
                                })
                            }
                            KeyCode::Char('c') | KeyCode::Char('3') => {
                                result = Some(if p.method.contains("permissions") {
                                    json!({"permissions":{},"scope":"turn"})
                                } else {
                                    json!({"decision":"cancel"})
                                })
                            }
                            _ => {}
                        }
                    } else {
                        match k.code {
                            KeyCode::Enter => match serde_json::from_str(input) {
                                Ok(v) => result = Some(v),
                                Err(e) => self.notice(format!("Enter a valid JSON response: {e}")),
                            },
                            KeyCode::Char(c) => input.push(c),
                            KeyCode::Backspace => pop(input),
                            _ => {}
                        }
                    }
                    if let Some(result) = result {
                        self.request(
                            "resolve",
                            json!({"requestId":request,"result":result,"generation":generation,"thread":thread}),
                            Effect::Generic,
                        );
                        keep = false;
                    }
                } else {
                    self.notice("This request has already been resolved");
                    keep = false;
                }
            }
        }
        if keep && self.modal.is_none() {
            self.modal = Some(modal)
        }
    }
    fn drop_target(&self, x: u16, y: u16) -> Option<(String, String)> {
        if let Some((_, Hit::Tab(k))) = self
            .hits
            .iter()
            .rev()
            .find(|(r, h)| matches!(h, Hit::Tab(_)) && contains(*r, x, y))
        {
            return Some((k.clone(), "center".into()));
        }
        let p = self.panes.iter().find(|p| contains(p.rect, x, y))?;
        if p.ids.is_empty() {
            return Some((self.root.clone(), "right".into()));
        }
        let target = p.ids.get(p.active)?.clone();
        let rx = x - p.rect.x;
        let ry = y - p.rect.y;
        let edge = if rx < p.rect.width / 5 {
            "left"
        } else if rx >= p.rect.width * 4 / 5 {
            "right"
        } else if ry < p.rect.height / 4 {
            "up"
        } else if ry >= p.rect.height * 3 / 4 {
            "down"
        } else {
            "center"
        };
        Some((target, edge.into()))
    }
    fn mouse(&mut self, m: MouseEvent) {
        if self.modal.is_some() {
            if matches!(self.modal, Some(Modal::Find { .. } | Modal::Tasks { .. })) {
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.modal_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                    }
                    MouseEventKind::ScrollDown => {
                        self.modal_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(hit) =
                            self.hits.iter().rev().find_map(|(r, h)| {
                                contains(*r, m.column, m.row).then_some(h.clone())
                            })
                        {
                            match hit {
                                Hit::Result(index)
                                    if matches!(self.modal, Some(Modal::Find { .. })) =>
                                {
                                    self.activate_result(index)
                                }
                                Hit::Task(key)
                                    if matches!(self.modal, Some(Modal::Tasks { .. })) =>
                                {
                                    self.modal = None;
                                    self.open(&key);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            if matches!(self.modal, Some(Modal::Models { .. })) {
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.modal_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                    }
                    MouseEventKind::ScrollDown => {
                        self.modal_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(index) = self.hits.iter().rev().find_map(|(r, hit)| match hit {
                            Hit::Model(index) if contains(*r, m.column, m.row) => Some(*index),
                            _ => None,
                        }) {
                            if let Some(Modal::Models { selected, .. }) = &mut self.modal {
                                *selected = index;
                            }
                            self.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        let x = m.column;
        let y = m.row;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(d) = self
                    .dividers
                    .iter()
                    .find(|d| contains(d.rect, x, y))
                    .cloned()
                {
                    self.remember();
                    self.drag = Some(Drag::Divider(d));
                    return;
                }
                if let Some((_, hit)) = self
                    .hits
                    .iter()
                    .rev()
                    .find(|(r, _)| contains(*r, x, y))
                    .cloned()
                {
                    match hit {
                        Hit::Find => self.find(""),
                        Hit::Tasks => {
                            self.modal = Some(Modal::Tasks {
                                query: String::new(),
                                selected: 0,
                                filter: 0,
                            })
                        }
                        Hit::ToggleSide => self.toggle_side(None),
                        Hit::Live(key) => self.live(&key),
                        Hit::Result(_) | Hit::Task(_) => {}
                        Hit::Model(_) => {}
                        Hit::Open(key) | Hit::Tab(key) => {
                            self.manager_selected = Some(ManagerTarget::Conversation(key.clone()));
                            self.drag = Some(Drag::Conversation { key, x, y });
                        }
                        Hit::New => self.new_form(),
                        Hit::Search => {
                            self.manager_focused = true;
                            self.manager_filtering = true;
                        }
                        Hit::ClearFilter => {
                            self.manager_query.clear();
                            self.manager_query_changed();
                            self.manager_focused = true;
                            self.manager_filtering = true;
                        }
                        Hit::ArchiveFilter => self.search(!self.manager_archived),
                        Hit::Group(path) => {
                            self.manager_focused = true;
                            self.manager_filtering = false;
                            self.toggle_directory(&path);
                        }
                        Hit::Side => self.side(String::new()),
                        Hit::Help => self.modal = Some(Modal::Help),
                        Hit::Pending => self.pending(),
                        Hit::Sidebar => self.drag = Some(Drag::Sidebar),
                    }
                } else if let Some(p) = self.panes.iter().find(|p| contains(p.rect, x, y)) {
                    if let Some(key) = p.ids.get(p.active).cloned() {
                        self.focus(&key);
                        self.persist();
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(drag) = self.drag.clone() {
                    match drag {
                        Drag::Divider(d) => {
                            let size = if d.vertical {
                                d.whole.height
                            } else {
                                d.whole.width
                            };
                            let n = if d.vertical {
                                y.saturating_sub(d.whole.y)
                            } else {
                                x.saturating_sub(d.whole.x)
                            };
                            if let Some(Node::Split { ratio, .. }) = self.layout.at_mut(&d.path) {
                                *ratio =
                                    ((n as u32 * 1000) / size.max(1) as u32).clamp(100, 900) as u16;
                            }
                        }
                        Drag::Sidebar => self.side_width = x.clamp(18, 52),
                        Drag::Conversation { key, .. } => {
                            if let Some((target, edge)) = self.drop_target(x, y) {
                                if key != target {
                                    self.notice(format!(
                                        "Drop → {edge} of {}",
                                        self.snap
                                            .data
                                            .conversations
                                            .get(&target)
                                            .map(|c| c.title.as_str())
                                            .unwrap_or(&target)
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(d) = self.drag.take() {
                    match d {
                        Drag::Conversation {
                            key,
                            x: oldx,
                            y: oldy,
                        } => {
                            if (x as i32 - oldx as i32).abs() + (y as i32 - oldy as i32).abs() < 2 {
                                self.open(&key)
                            } else if let Some((target, edge)) = self.drop_target(x, y) {
                                self.dock(&key, &target, &edge)
                            } else if let Some((_, Hit::Open(target))) = self
                                .hits
                                .iter()
                                .find(|(r, h)| matches!(h, Hit::Open(_)) && contains(*r, x, y))
                                .cloned()
                            {
                                self.request(
                                    "reorder",
                                    json!({"source":key,"target":target}),
                                    Effect::Generic,
                                )
                            }
                        }
                        _ => self.persist(),
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if self.sidebar_rect.is_some_and(|r| contains(r, x, y)) {
                    self.manager_reveal = false;
                    if up {
                        self.side_scroll = self.side_scroll.saturating_sub(3)
                    } else {
                        self.side_scroll =
                            (self.side_scroll + 3).min(self.rows().len().saturating_sub(3))
                    }
                } else {
                    if let Some(p) = self.panes.iter().find(|p| contains(p.rect, x, y)) {
                        if let Some(k) = p.ids.get(p.active).cloned() {
                            self.focus = k;
                        }
                    }
                    self.scroll(if up { 3 } else { -3 });
                }
            }
            _ => {}
        }
    }
    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::default().style(Style::default().bg(BG).fg(FG)), area);
        self.hits.clear();
        self.sidebar_rect = None;
        if area.width < 20 || area.height < 8 {
            return;
        }
        let running = self
            .snap
            .data
            .conversations
            .values()
            .filter(|c| c.busy())
            .count();
        let status = if self.demo {
            "DEMO"
        } else if self.snap.connected {
            "CONNECTED"
        } else {
            "OFFLINE"
        };
        let header = Line::from(vec![
            Span::styled(" ◆ Codex Deck ", style(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(
                    " {status}  ·  {running} running  ·  {} requests",
                    self.snap.pending.len()
                ),
                style(MUTED),
            ),
        ]);
        f.render_widget(
            Paragraph::new(header).style(Style::default().bg(PANEL)),
            Rect::new(0, 0, area.width, 2),
        );
        if area.width > 72 {
            let r = Rect::new(area.width - 22, 0, 22, 1);
            f.render_widget(
                Paragraph::new("[ Approvals ] [ Help ]").style(style(ACCENT)),
                r,
            );
            self.hits.push((Rect::new(r.x, 0, 13, 1), Hit::Pending));
            self.hits.push((Rect::new(r.x + 14, 0, 8, 1), Hit::Help));
        }
        for (x, label, hit) in [
            (0, "[ Tasks ]", Hit::Tasks),
            (11, "[ Find messages ]", Hit::Find),
            (
                30,
                if self.side_visible() {
                    "[ Hide side ]"
                } else {
                    "[ Show side ]"
                },
                Hit::ToggleSide,
            ),
        ] {
            if area.width >= x + label.width() as u16 {
                let rect = Rect::new(x, 1, label.width() as u16, 1);
                f.render_widget(Paragraph::new(label).style(style(ACCENT)), rect);
                self.hits.push((rect, hit));
            }
        }
        let content = Rect::new(0, 2, area.width, area.height.saturating_sub(4));
        let manager_only = self.sidebar && self.manager_focused && area.width < 44;
        let use_side = self.sidebar && (area.width >= 44 || manager_only);
        let main = if manager_only {
            self.sidebar_rect = Some(content);
            self.draw_sidebar(f, content);
            Rect::new(area.width, content.y, 0, content.height)
        } else if use_side {
            let w = self.side_width.min((area.width / 3).max(18));
            let sidebar = Rect::new(0, content.y, w, content.height);
            self.sidebar_rect = Some(sidebar);
            self.draw_sidebar(f, sidebar);
            let boundary = Rect::new(w, content.y, 1, content.height);
            f.render_widget(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(style(BORDER)),
                boundary,
            );
            self.hits.push((boundary, Hit::Sidebar));
            Rect::new(w + 1, content.y, area.width - w - 1, content.height)
        } else {
            content
        };
        let layout = if self.collapsed_side_panels.contains(&self.root) {
            Node::single(&self.root)
        } else {
            self.layout.clone()
        };
        let (mut panes, dividers) = layout.panes(main);
        // A compact terminal presents the focused conversation as the active tab without modifying the saved tree.
        for p in &mut panes {
            if p.ids.contains(&self.focus) {
                p.active = p
                    .ids
                    .iter()
                    .position(|k| k == &self.focus)
                    .unwrap_or(p.active);
            }
        }
        self.panes = panes.clone();
        self.dividers = dividers.clone();
        for d in &dividers {
            f.render_widget(
                Block::default()
                    .borders(if d.vertical {
                        Borders::TOP
                    } else {
                        Borders::LEFT
                    })
                    .border_style(style(BORDER)),
                d.rect,
            );
        }
        for p in &panes {
            if p.ids.is_empty() {
                f.render_widget(Paragraph::new("SIDE CONVERSATION\n\n[ Open side ]\n\n/side <question>\nor /btw <question>\n\nRead-only discussion").style(style(MUTED)).block(Block::default().padding(ratatui::widgets::Padding::uniform(1))), p.rect);
                let button = Rect::new(
                    p.rect.x + 1,
                    p.rect.y + 3,
                    p.rect.width.saturating_sub(2),
                    1,
                );
                self.hits.push((button, Hit::Side));
            } else {
                self.draw_pane(f, p);
            }
        }
        if self.focus.is_empty() {
            let welcome = panes.first().map(|p| p.rect).unwrap_or(main);
            f.render_widget(Paragraph::new("Your conversations, across directories.\n\nCtrl+G n  New conversation\nCtrl+G p  Filter the left conversation manager\n/side or /btw  Open a side conversation\nF1  All commands\n\nStart Codex CLI first if you need to sign in.\nUse /login to start device login.").style(style(MUTED)).block(Block::default().padding(ratatui::widgets::Padding::uniform(1))),welcome);
        }
        let message = if self.prefix {
            "Ctrl+G → n new · p manager · f quick switch · s side · m move · r resize · z maximize · w close · a requests · q detach · ? help".into()
        } else if self.resize {
            "RESIZE · arrows adjust split · Enter/Esc finish".into()
        } else if self.manager_focused {
            let path = match &self.manager_selected {
                Some(ManagerTarget::Directory(path)) => path.clone(),
                Some(ManagerTarget::Conversation(key)) => self
                    .snap
                    .data
                    .conversations
                    .get(key)
                    .map(|c| c.cwd.clone())
                    .unwrap_or_default(),
                None => String::new(),
            };
            format!("{path} · ↑↓ select · Enter open/fold · Esc clear · Tab return")
        } else if self.toast_at.elapsed() < Duration::from_secs(10) && !self.toast.is_empty() {
            self.toast.clone()
        } else if !self.snap.error.is_empty() {
            self.snap.error.clone()
        } else {
            format!(
                "{}  ·  Ctrl+G commands  ·  F1 help  ·  PgUp/PgDn history",
                self.snap
                    .data
                    .conversations
                    .get(&self.focus)
                    .map(|c| format!("{} · {}", c.status, c.cwd))
                    .unwrap_or_else(|| "Ready".into())
            )
        };
        f.render_widget(
            Paragraph::new(short(&message, area.width as usize)).style(
                Style::default()
                    .fg(if self.snap.connected {
                        MUTED
                    } else {
                        Color::Yellow
                    })
                    .bg(PANEL),
            ),
            Rect::new(0, area.height - 2, area.width, 2),
        );
        if let Some(modal) = self.modal.clone() {
            self.draw_modal(f, modal)
        }
    }
    fn draw_sidebar(&mut self, f: &mut Frame, r: Rect) {
        f.render_widget(Block::default().style(Style::default().bg(PANEL)), r);
        let accent = if self.manager_focused { ACCENT } else { FG };
        f.render_widget(
            Paragraph::new(" CONVERSATIONS").style(style(accent).add_modifier(Modifier::BOLD)),
            Rect::new(r.x, r.y, r.width, 1),
        );
        if r.height < 6 {
            return;
        }
        let new = Rect::new(r.x, r.y + 1, r.width, 1);
        f.render_widget(
            Paragraph::new(" + New conversation").style(style(ACCENT)),
            new,
        );
        self.hits.push((new, Hit::New));
        let mode = Rect::new(r.x, r.y + 2, r.width, 1);
        f.render_widget(
            Paragraph::new(if self.manager_archived {
                " Archived · show active"
            } else {
                " Active · show archived"
            })
            .style(style(MUTED)),
            mode,
        );
        self.hits.push((mode, Hit::ArchiveFilter));
        let filter = Rect::new(r.x + 1, r.y + 3, r.width.saturating_sub(2), 1);
        let filter_width = filter.width.saturating_sub(4) as usize;
        let query = if self.manager_query.is_empty() {
            "Filter chats / paths".into()
        } else {
            path_tail(&self.manager_query, filter_width)
        };
        f.render_widget(Block::default().style(Style::default().bg(SELECT)), filter);
        f.render_widget(
            Paragraph::new(short(&query, filter_width)).style(
                Style::default()
                    .fg(if self.manager_filtering && self.manager_focused {
                        ACCENT
                    } else {
                        MUTED
                    })
                    .bg(SELECT),
            ),
            Rect::new(filter.x, filter.y, filter_width as u16, 1),
        );
        f.render_widget(
            Paragraph::new("[x]").style(style(MUTED).bg(SELECT)),
            Rect::new(filter.right().saturating_sub(3), filter.y, 3, 1),
        );
        self.hits.push((filter, Hit::Search));
        self.hits.push((
            Rect::new(filter.right().saturating_sub(3), filter.y, 3, 1),
            Hit::ClearFilter,
        ));
        let rows = self.rows();
        if self.manager_focused
            && !rows
                .iter()
                .any(|r| r.target.is_some() && r.target == self.manager_selected)
        {
            self.manager_selected = rows.iter().find_map(|row| row.target.clone());
        }
        let visible = r.height.saturating_sub(6) as usize;
        if self.manager_reveal {
            if let Some(selected) = rows
                .iter()
                .position(|row| row.target.is_some() && row.target == self.manager_selected)
            {
                if selected < self.side_scroll {
                    self.side_scroll = selected;
                } else if selected >= self.side_scroll + visible {
                    self.side_scroll = selected.saturating_sub(visible.saturating_sub(1));
                }
            }
            self.manager_reveal = false;
        }
        self.side_scroll = self.side_scroll.min(rows.len().saturating_sub(visible));
        if rows.is_empty() {
            f.render_widget(
                Paragraph::new("No matching conversations")
                    .style(style(MUTED))
                    .wrap(Wrap { trim: false }),
                Rect::new(r.x + 1, r.y + 5, r.width.saturating_sub(2), visible as u16),
            );
        }
        for (i, row) in rows
            .into_iter()
            .skip(self.side_scroll)
            .take(visible)
            .enumerate()
        {
            let rr = Rect::new(r.x + 1, r.y + 5 + i as u16, r.width.saturating_sub(2), 1);
            let selected =
                self.manager_focused && row.target.is_some() && row.target == self.manager_selected;
            let active =
                matches!(&row.target, Some(ManagerTarget::Conversation(key)) if key == &self.focus);
            let header = matches!(&row.target, Some(ManagerTarget::Directory(_)));
            let title = if row.target.is_none() {
                path_tail(&row.title, rr.width as usize)
            } else {
                short(&row.title, rr.width as usize)
            };
            f.render_widget(
                Paragraph::new(title).style(
                    Style::default()
                        .fg(if selected || active {
                            ACCENT
                        } else if row.target.is_none() {
                            MUTED
                        } else {
                            FG
                        })
                        .bg(if selected || active { SELECT } else { PANEL })
                        .add_modifier(if header {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                rr,
            );
            match row.target {
                Some(ManagerTarget::Conversation(key)) => self.hits.push((rr, Hit::Open(key))),
                Some(ManagerTarget::Directory(path)) => self.hits.push((rr, Hit::Group(path))),
                None => {}
            }
        }
        let footer = Rect::new(r.x + 1, r.bottom() - 1, r.width.saturating_sub(2), 1);
        f.render_widget(
            Paragraph::new(short("↑↓ select · Enter open", footer.width as usize))
                .style(style(MUTED)),
            footer,
        );
        if self.manager_focused && self.manager_filtering && self.modal.is_none() && !self.prefix {
            let shown = path_tail(&self.manager_query, filter_width);
            f.set_cursor_position((
                filter.x + shown.width().min(filter_width.saturating_sub(1)) as u16,
                filter.y,
            ));
        }
    }
    fn draw_pane(&mut self, f: &mut Frame, p: &Pane) {
        let r = p.rect;
        if r.width < 4 || r.height < 6 {
            return;
        }
        let Some(key) = p.ids.get(p.active) else {
            return;
        };
        let active = key == &self.focus && !self.manager_focused;
        let color = if active { ACCENT } else { BORDER };
        let c = self.snap.data.conversations.get(key);
        let mut x = r.x;
        for id in &p.ids {
            let title = self
                .snap
                .data
                .conversations
                .get(id)
                .map(|c| c.title.as_str())
                .unwrap_or("Loading…");
            let w = (title.width() + 4)
                .min(28)
                .min(r.right().saturating_sub(x) as usize) as u16;
            if w == 0 {
                break;
            }
            let tab = Rect::new(x, r.y, w, 1);
            f.render_widget(
                Paragraph::new(format!(" {} ", short(title, w.saturating_sub(2) as usize))).style(
                    Style::default()
                        .fg(if id == key { ACCENT } else { MUTED })
                        .bg(if id == key { SELECT } else { PANEL }),
                ),
                tab,
            );
            self.hits.push((tab, Hit::Tab(id.clone())));
            x += w;
        }
        let info = if let Some(c) = &c {
            format!(
                "{} · {} · {}{}",
                if c.readonly {
                    "Discussion / read only"
                } else {
                    "Execute / workspace write"
                },
                if c.model.is_empty() {
                    "configured model"
                } else {
                    &c.model
                },
                if self.snap.loading.contains(key) {
                    "loading history…"
                } else {
                    &c.status
                },
                if c.parent.is_some() {
                    " · /parent"
                } else {
                    ""
                }
            )
        } else {
            "Loading conversation…".into()
        };
        f.render_widget(
            Paragraph::new(short(&info, r.width.saturating_sub(2) as usize)).style(style(MUTED)),
            Rect::new(r.x + 1, r.y + 1, r.width - 2, 1),
        );
        if self.search_views.contains_key(key) {
            let banner = Rect::new(r.x + 1, r.y + 2, r.width - 2, 1);
            f.render_widget(
                Paragraph::new(short(
                    "Search context · /live returns to latest",
                    banner.width as usize,
                ))
                .style(style(ACCENT)),
                banner,
            );
            self.hits.push((banner, Hit::Live(key.clone())));
        } else if let Some(c) = &c {
            f.render_widget(
                Paragraph::new(short(
                    &format!("cwd for next task: {}", c.cwd),
                    r.width.saturating_sub(2) as usize,
                ))
                .style(style(MUTED)),
                Rect::new(r.x + 1, r.y + 2, r.width - 2, 1),
            );
        }
        let d = self.drafts.get(key).cloned().unwrap_or_default();
        let input_h = (d.text.lines().count() as u16 + 2)
            .clamp(3, 6)
            .min(r.height.saturating_sub(5));
        let input = Rect::new(r.x, r.bottom() - input_h, r.width, input_h);
        let messages = Rect::new(r.x, r.y + 3, r.width, r.height.saturating_sub(input_h + 3));
        let inner_messages = Rect::new(
            messages.x + 1,
            messages.y,
            messages.width.saturating_sub(2),
            messages.height,
        );
        let transcript = self.transcripts.entry(key.clone()).or_default();
        let display = self.search_views.get(key).or(c);
        if let Some(c) = display {
            let parent = c.parent.as_ref().map(|parent| {
                self.snap
                    .data
                    .conversations
                    .get(parent)
                    .map(|c| c.title.as_str())
                    .unwrap_or(parent)
            });
            transcript.update(c, parent, inner_messages.width);
        }
        let total = transcript.height;
        let first = display
            .as_ref()
            .and_then(|c| c.messages.first())
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let off = self.scroll.entry(key.clone()).or_default();
        let scroll_before = *off;
        if let Some((old, old_first)) = self.heights.get(key) {
            if *off > 0 && &first == old_first {
                *off = (*off as i64 + total as i64 - *old as i64).clamp(1, 65535) as u16;
            }
        }
        if let Some(target) = self.jump_targets.remove(key) {
            let mut position = transcript.prefix.height;
            for cached in &transcript.messages {
                if cached.source.id == target {
                    break;
                }
                position += cached.rendered.height;
            }
            *off = total
                .saturating_sub(messages.height as usize)
                .saturating_sub(position)
                .min(65535) as u16;
        }
        let off = *off;
        self.heights.insert(key.clone(), (total, first));
        let start = total
            .saturating_sub(messages.height as usize)
            .saturating_sub(off as usize);
        transcript.render(f, inner_messages, start);
        let busy = c.is_some_and(Conversation::busy);
        let title = if busy {
            " Message · Enter queues · /steer to redirect "
        } else {
            " Message · Enter send · Ctrl+J newline "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(style(color))
            .title(short(title, input.width.saturating_sub(2) as usize));
        let inner = block.inner(input);
        let mut para = Paragraph::new(d.text.clone())
            .block(block)
            .wrap(Wrap { trim: false });
        let before = &d.text[..safe_cursor(&d)];
        let (cx, cy) = cursor_cell(before, inner.width.max(1));
        let first = cy.saturating_sub(inner.height.saturating_sub(1));
        para = para.scroll((first, 0));
        f.render_widget(para, input);
        if active && self.modal.is_none() && !self.prefix {
            f.set_cursor_position((
                inner.x + cx.min(inner.width.saturating_sub(1)),
                inner.y + (cy - first).min(inner.height.saturating_sub(1)),
            ));
        }
        if scroll_before != off {
            self.fire("scroll", json!({"thread":key,"offset":off}));
        }
    }
    fn draw_models(&mut self, f: &mut Frame, r: Rect, thread: &str, query: &str, selected: usize) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(style(ACCENT))
            .title(" Select model · Esc cancels ");
        let inner = block.inner(r);
        f.render_widget(block, r);
        if inner.height < 10 || inner.width < 20 {
            f.render_widget(Paragraph::new("Enlarge terminal to select a model"), inner);
            return;
        }
        let conversation = self.snap.data.conversations.get(thread);
        let current = conversation.map(|c| c.model.as_str()).unwrap_or("");
        let busy = conversation.is_some_and(|c| c.busy() || !c.queued.is_empty());
        f.render_widget(
            Paragraph::new(short(
                &format!(
                    "Conversation: {}",
                    conversation.map(|c| c.title.as_str()).unwrap_or(thread)
                ),
                inner.width as usize,
            ))
            .style(style(MUTED)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        f.render_widget(
            Paragraph::new(short(
                &format!(
                    "Current: {}",
                    if current.is_empty() {
                        "configured default"
                    } else {
                        current
                    }
                ),
                inner.width as usize,
            ))
            .style(style(ACCENT)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        f.render_widget(
            Paragraph::new(format!(
                "Filter: {}▏",
                path_tail(query, inner.width.saturating_sub(9) as usize)
            ))
            .style(style(FG)),
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
        );
        f.render_widget(
            Paragraph::new(short(
                if busy {
                    "Stop the task and clear its queue before changing the model"
                } else {
                    "↑↓ / Tab select · Enter / click apply · type to filter"
                },
                inner.width as usize,
            ))
            .style(style(if busy { Color::Yellow } else { MUTED })),
            Rect::new(inner.x, inner.y + 3, inner.width, 1),
        );
        let choices = self.model_choices(query);
        let selected = selected.min(choices.len().saturating_sub(1));
        let capacity = inner.height.saturating_sub(8) as usize / 2;
        let start = selected.saturating_sub(capacity.saturating_sub(1));
        for (index, model) in choices.iter().enumerate().skip(start).take(capacity) {
            let y = inner.y + 5 + ((index - start) * 2) as u16;
            let active = model.value == current || (current.is_empty() && model.default);
            let badges = match (active, model.default) {
                (true, true) => " · current · default",
                (true, false) => " · current",
                (false, true) => " · default",
                (false, false) => "",
            };
            let row_style = if index == selected {
                Style::default().fg(ACCENT).bg(SELECT)
            } else {
                style(FG)
            };
            let name = model.name.replace(['\r', '\n', '\t'], " ");
            f.render_widget(
                Paragraph::new(short(
                    &format!(
                        "{} {name}{badges}",
                        if index == selected { "›" } else { " " }
                    ),
                    inner.width as usize,
                ))
                .style(row_style),
                Rect::new(inner.x, y, inner.width, 1),
            );
            f.render_widget(
                Paragraph::new(short(&format!("  {}", model.value), inner.width as usize))
                    .style(row_style.fg(MUTED)),
                Rect::new(inner.x, y + 1, inner.width, 1),
            );
            self.hits
                .push((Rect::new(inner.x, y, inner.width, 2), Hit::Model(index)));
        }
        if choices.is_empty() {
            f.render_widget(
                Paragraph::new(if self.snap.models.is_empty() {
                    "No models returned. Use /model <id> for a custom model."
                } else {
                    "No matching models. Backspace or Ctrl+U clears the filter."
                })
                .wrap(Wrap { trim: false })
                .style(style(MUTED)),
                Rect::new(
                    inner.x,
                    inner.y + 5,
                    inner.width,
                    inner.height.saturating_sub(8),
                ),
            );
        } else if let Some(model) = choices.get(selected) {
            f.render_widget(
                Paragraph::new(model.description.as_str())
                    .wrap(Wrap { trim: false })
                    .style(style(MUTED)),
                Rect::new(inner.x, inner.bottom() - 3, inner.width, 2),
            );
        }
        let footer = if choices.is_empty() {
            "Esc cancels · /model <id> sets a custom model".into()
        } else {
            format!(
                "{} / {} · PgUp/PgDn scroll · Esc cancels",
                selected + 1,
                choices.len()
            )
        };
        f.render_widget(
            Paragraph::new(short(&footer, inner.width as usize)).style(style(MUTED)),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
    }
    fn draw_find(&mut self, f: &mut Frame, r: Rect, query: &str, submitted: &str, selected: usize) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(style(ACCENT))
            .title(" Find message text · Esc cancels ");
        let a = block.inner(r);
        f.render_widget(block, r);
        if a.height < 9 {
            return;
        }
        let current = query == submitted && self.snap.search.query == query;
        let state = &self.snap.search;
        let status = if !current {
            "Enter to search all directories and archived chats".into()
        } else if state.running {
            format!(
                "Refreshing index {}/{} · {} matches so far",
                state.completed,
                state.total,
                state.hits.len()
            )
        } else {
            format!(
                "{} matches · {} histories checked · {} read errors{}",
                state.hits.len(),
                state.completed,
                state.errors,
                if state.truncated {
                    " · first 100 shown"
                } else {
                    ""
                }
            )
        };
        for (row, line) in [
            format!(
                "Text: {}▏",
                path_tail(query, a.width.saturating_sub(8) as usize)
            ),
            status,
            "Enter search / open · ↑↓ select · Ctrl+R refresh · /live exits context".into(),
        ]
        .into_iter()
        .enumerate()
        {
            f.render_widget(
                Paragraph::new(short(&line, a.width as usize)).style(style(if row == 0 {
                    ACCENT
                } else {
                    MUTED
                })),
                Rect::new(a.x, a.y + row as u16, a.width, 1),
            );
        }
        if current {
            let capacity = (a.height.saturating_sub(6) as usize / 3).max(1);
            let selected = selected.min(state.hits.len().saturating_sub(1));
            let start = selected.saturating_sub(capacity - 1);
            for (i, hit) in state.hits.iter().enumerate().skip(start).take(capacity) {
                let y = a.y + 4 + ((i - start) * 3) as u16;
                let s = if i == selected {
                    Style::default().fg(ACCENT).bg(SELECT)
                } else {
                    style(FG)
                };
                for (dy, line) in [
                    format!(
                        "{} {} · {}{}",
                        if i == selected { "›" } else { " " },
                        hit.title,
                        hit.role,
                        if hit.archived { " · archived" } else { "" }
                    ),
                    hit.cwd.clone(),
                    hit.excerpt.clone(),
                ]
                .into_iter()
                .enumerate()
                {
                    f.render_widget(
                        Paragraph::new(short(&line, a.width as usize)).style(s),
                        Rect::new(a.x, y + dy as u16, a.width, 1),
                    );
                }
                self.hits
                    .push((Rect::new(a.x, y, a.width, 3), Hit::Result(i)));
            }
            if state.hits.is_empty() {
                f.render_widget(
                    Paragraph::new(if state.running {
                        "Reading history in the background…"
                    } else {
                        "No matching message text"
                    })
                    .style(style(MUTED)),
                    Rect::new(a.x, a.y + 4, a.width, 1),
                );
            }
            if !state.error.is_empty() {
                f.render_widget(
                    Paragraph::new(short(&state.error, a.width as usize))
                        .style(style(Color::Yellow)),
                    Rect::new(a.x, a.bottom() - 2, a.width, 1),
                );
            }
        }
        f.render_widget(
            Paragraph::new("Literal text · messages and tool output · no model request")
                .style(style(MUTED)),
            Rect::new(a.x, a.bottom() - 1, a.width, 1),
        );
    }
    fn draw_tasks(&mut self, f: &mut Frame, r: Rect, query: &str, selected: usize, filter: usize) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(style(ACCENT))
            .title(" Background tasks · live · Esc closes ");
        let a = block.inner(r);
        f.render_widget(block, r);
        if a.height < 9 {
            return;
        }
        let mut counts = [0; 5];
        for c in self.snap.data.conversations.values() {
            counts[self.task_kind(c)] += 1;
        }
        let summary = format!(
            "Running {} · queued {} · attention {} · failed / unknown {}",
            counts[1], counts[2], counts[3], counts[4]
        );
        for (dy, line) in [
            summary,
            format!(
                "Filter: {} · {}▏",
                ["All", "Running", "Queued", "Attention", "Failed / unknown"][filter],
                path_tail(query, a.width.saturating_sub(24) as usize)
            ),
            "Tab changes status · ↑↓ select · Enter / click open · F2 requests".into(),
        ]
        .into_iter()
        .enumerate()
        {
            f.render_widget(
                Paragraph::new(short(&line, a.width as usize)).style(style(if dy == 1 {
                    ACCENT
                } else {
                    MUTED
                })),
                Rect::new(a.x, a.y + dy as u16, a.width, 1),
            );
        }
        let rows = self.task_rows(query, filter);
        let selected = selected.min(rows.len().saturating_sub(1));
        let capacity = (a.height.saturating_sub(6) as usize / 3).max(1);
        let start = selected.saturating_sub(capacity - 1);
        for (i, key) in rows.iter().enumerate().skip(start).take(capacity) {
            let c = &self.snap.data.conversations[key];
            let y = a.y + 4 + ((i - start) * 3) as u16;
            let requests = self
                .snap
                .pending
                .iter()
                .filter(|p| p.thread == *key)
                .count();
            let held = c.queued.iter().filter(|q| q.state != "queued").count();
            let s = if i == selected {
                Style::default().fg(ACCENT).bg(SELECT)
            } else {
                style(FG)
            };
            for (dy, line) in [
                format!("{} {}", if i == selected { "›" } else { " " }, c.title),
                format!(
                    "{} · queue {} ({} held) · {} requests",
                    c.status,
                    c.queued.len(),
                    held,
                    requests
                ),
                c.cwd.clone(),
            ]
            .into_iter()
            .enumerate()
            {
                f.render_widget(
                    Paragraph::new(short(&line, a.width as usize)).style(s),
                    Rect::new(a.x, y + dy as u16, a.width, 1),
                );
            }
            self.hits
                .push((Rect::new(a.x, y, a.width, 3), Hit::Task(key.clone())));
        }
        if rows.is_empty() {
            f.render_widget(
                Paragraph::new("No tasks in this view").style(style(MUTED)),
                Rect::new(a.x, a.y + 4, a.width, 1),
            );
        }
        let detail = rows
            .get(selected)
            .and_then(|key| self.snap.data.conversations[key].queued.first())
            .map(|q| format!("Next [{}]: {}", q.state, q.text.replace(['\n', '\r'], " ")))
            .unwrap_or_else(|| {
                "Opening a task preserves its queue and background execution".into()
            });
        f.render_widget(
            Paragraph::new(short(&detail, a.width as usize)).style(style(MUTED)),
            Rect::new(a.x, a.bottom() - 1, a.width, 1),
        );
    }
    fn draw_modal(&mut self, f: &mut Frame, m: Modal) {
        let a = f.area();
        let w = a.width.saturating_sub(6).min(100);
        let h = a.height.saturating_sub(4);
        let r = Rect::new((a.width - w) / 2, (a.height - h) / 2, w, h);
        f.render_widget(Clear, r);
        let title: String;
        let mut lines: Vec<Line> = vec![];
        let mut offset = 0;
        match m {
            Modal::Find {
                query,
                submitted,
                selected,
            } => {
                self.draw_find(f, r, &query, &submitted, selected);
                return;
            }
            Modal::Tasks {
                query,
                selected,
                filter,
            } => {
                self.draw_tasks(f, r, &query, selected, filter);
                return;
            }
            Modal::Models {
                thread,
                query,
                selected,
            } => {
                self.draw_models(f, r, &thread, &query, selected);
                return;
            }
            Modal::Help => {
                title = "Codex Deck · keyboard & commands · Esc closes".into();
                for line in HELP.lines() {
                    lines.push(Line::styled(line.to_string(), style(FG)));
                }
            }
            Modal::Text {
                title: t,
                body,
                scroll,
            } => {
                title = t;
                offset = scroll;
                lines = body.lines().map(|s| Line::raw(s.to_string())).collect();
            }
            Modal::Search {
                query,
                selected,
                archived,
            } => {
                title = format!(
                    "{} · Tab toggles archive · Enter opens",
                    if archived {
                        "Archived"
                    } else {
                        "Switch conversation"
                    }
                );
                lines.push(Line::styled(format!("> {query}▏"), style(ACCENT)));
                lines.push(Line::styled(
                    "Search title, directory, project or thread ID",
                    style(MUTED),
                ));
                lines.push(Line::default());
                let keys = self.sorted(archived, &query);
                let start = selected.saturating_sub(h.saturating_sub(8) as usize);
                for (i, k) in keys
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(h.saturating_sub(6) as usize)
                {
                    let c = &self.snap.data.conversations[k];
                    lines.push(Line::styled(
                        format!(
                            "{} {} · {} · {}",
                            if i == selected { "›" } else { " " },
                            c.title,
                            c.cwd,
                            short(k, 8)
                        ),
                        if i == selected {
                            Style::default().fg(ACCENT).bg(SELECT)
                        } else {
                            style(FG)
                        },
                    ));
                }
            }
            Modal::New {
                fields,
                focus,
                readonly,
            } => {
                title = "New conversation · Enter creates local draft · Tab next field".into();
                lines.push(Line::styled(
                    "The backend thread is created when you send the first message.",
                    style(MUTED),
                ));
                lines.push(Line::default());
                for (i, label) in ["Directory", "Title (optional)", "Model ID (optional)"]
                    .iter()
                    .enumerate()
                {
                    lines.push(Line::styled(
                        format!("{} {}", if focus == i { "›" } else { " " }, label),
                        style(if focus == i { ACCENT } else { MUTED }),
                    ));
                    lines.push(Line::raw(format!(
                        "  {}{}",
                        fields[i],
                        if focus == i { "▏" } else { "" }
                    )));
                    lines.push(Line::default());
                }
                lines.push(Line::styled(
                    format!(
                        "Ctrl+R: {}",
                        if readonly {
                            "Discussion · read only"
                        } else {
                            "Execute · workspace write"
                        }
                    ),
                    style(ACCENT),
                ));
            }
            Modal::Prompt {
                title: t, value, ..
            } => {
                title = "Input · Enter confirms · Esc cancels".into();
                lines.push(Line::styled(t, style(ACCENT)));
                lines.push(Line::default());
                lines.push(Line::raw(format!("{value}▏")));
            }
            Modal::Move { selected, edge } => {
                title = "Move focused conversation · ↑↓ target · ←→ edge · Enter apply".into();
                lines.push(Line::styled(
                    format!(
                        "Dock: {}",
                        ["RIGHT", "LEFT", "TOP", "BOTTOM", "TAB GROUP"][edge]
                    ),
                    style(ACCENT),
                ));
                lines.push(Line::default());
                let mut ids = vec![];
                self.layout.ids(&mut ids);
                ids.retain(|k| k != &self.focus);
                for (i, id) in ids.iter().enumerate() {
                    lines.push(Line::styled(
                        format!(
                            "{} {}",
                            if i == selected { "›" } else { " " },
                            self.snap
                                .data
                                .conversations
                                .get(id)
                                .map(|c| c.title.as_str())
                                .unwrap_or(id)
                        ),
                        style(if i == selected { ACCENT } else { FG }),
                    ));
                }
                if ids.is_empty() {
                    lines.push(Line::raw(
                        "Open /side or drag another conversation here first.",
                    ));
                }
            }
            Modal::Pending {
                request,
                question,
                input,
                scroll,
                generation,
                thread,
                ..
            } => {
                offset = scroll;
                title = "Request · Esc leaves it pending".into();
                if let Some(p) =
                    self.snap.pending.iter().find(|p| {
                        p.id == request && p.generation == generation && p.thread == thread
                    })
                {
                    let c = self.snap.data.conversations.get(&p.thread);
                    lines.push(Line::styled(
                        format!(
                            "Conversation: {}",
                            c.map(|c| c.title.as_str()).unwrap_or(&p.thread)
                        ),
                        style(ACCENT),
                    ));
                    lines.push(Line::styled(format!("Thread: {}", p.thread), style(MUTED)));
                    if let Some(c) = c {
                        lines.push(Line::styled(format!("Directory: {}", c.cwd), style(MUTED)));
                    }
                    lines.push(Line::raw(p.method.clone()));
                    lines.push(Line::default());
                    if p.method.ends_with("requestUserInput") {
                        if let Some(q) = p.params["questions"]
                            .as_array()
                            .and_then(|a| a.get(question))
                        {
                            lines.push(Line::raw(text(q, "question")));
                            if let Some(options) = q["options"].as_array() {
                                for (i, o) in options.iter().enumerate() {
                                    lines.push(Line::raw(format!(
                                        "{}. {} · {}",
                                        i + 1,
                                        text(o, "label"),
                                        text(o, "description")
                                    )));
                                }
                            }
                            lines.push(Line::default());
                            lines.push(Line::styled(
                                format!("Answer (number or text): {input}▏"),
                                style(ACCENT),
                            ));
                        }
                    } else {
                        let details = serde_json::to_string_pretty(&p.params).unwrap_or_default();
                        if p.method.ends_with("requestApproval") {
                            lines.push(Line::styled(
                                "[1 / y] Allow once  [2 / n] Deny  [3 / c] Cancel",
                                style(ACCENT),
                            ));
                        } else {
                            lines.push(Line::styled(
                                "Enter an explicit JSON response for this server request:",
                                style(ACCENT),
                            ));
                            lines.push(Line::raw(format!("{input}▏")));
                        }
                        lines.push(Line::default());
                        for s in details.lines() {
                            lines.push(Line::raw(s.to_string()));
                        }
                    }
                } else {
                    lines.push(Line::raw("Request resolved. Press Esc."));
                }
            }
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(style(ACCENT))
            .title(short(&title, w.saturating_sub(2) as usize))
            .padding(ratatui::widgets::Padding::uniform(1))
            .style(Style::default().bg(PANEL).fg(FG));
        f.render_widget(
            Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((offset, 0)),
            r,
        );
    }
}
fn pop(s: &mut String) {
    if let Some((i, _)) = s.grapheme_indices(true).next_back() {
        s.truncate(i)
    }
}
fn vertical_cursor(d: &mut Draft, down: bool) {
    let cursor = safe_cursor(d);
    let start = d.text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let column = d.text[start..cursor].width();
    let end = d.text[cursor..]
        .find('\n')
        .map_or(d.text.len(), |i| cursor + i);
    let range = if down {
        if end == d.text.len() {
            return;
        }
        (
            end + 1,
            d.text[end + 1..]
                .find('\n')
                .map_or(d.text.len(), |i| end + 1 + i),
        )
    } else {
        if start == 0 {
            return;
        }
        (
            d.text[..start - 1].rfind('\n').map_or(0, |i| i + 1),
            start - 1,
        )
    };
    let mut n = 0;
    d.cursor = range.0;
    for (i, g) in d.text[range.0..range.1].grapheme_indices(true) {
        if n + g.width() > column {
            break;
        }
        n += g.width();
        d.cursor = range.0 + i + g.len();
    }
}
fn safe_cursor(d: &Draft) -> usize {
    let mut i = d.cursor.min(d.text.len());
    while !d.text.is_char_boundary(i) {
        i -= 1
    }
    i
}
fn cursor_cell(s: &str, width: u16) -> (u16, u16) {
    let (mut x, mut y) = (0, 0);
    for g in s.graphemes(true) {
        if g == "\n" {
            x = 0;
            y += 1;
            continue;
        }
        let n = g.width() as u16;
        if x + n > width {
            x = 0;
            y += 1
        }
        x += n;
        if x >= width {
            x = 0;
            y += 1
        }
    }
    (x, y)
}
const HELP:&str="GET STARTED\nCtrl+G n  New draft       Ctrl+G p  Focus left filter\nEnter sends; during a task it queues the next message.\nCtrl+J newline · paste supports multiline · Ctrl+C stop focused task\nCtrl+G q / Ctrl+D detaches. Background tasks keep running.\n\nCONVERSATION MANAGER\nLeft: directories and chats · middle: main · right: side\nClick a chat to open; click a directory to fold/unfold.\nCtrl+G p focuses the inline filter · arrows select · Enter open\nEsc clears the filter · Tab returns to chat · Ctrl+G f quick switch\n\nWORKSPACE · press Ctrl+G, release, then the key\ns side · o hide/show right · b sidebar · arrows / h j k l focus\nt tasks · / find message text · [ ] cycle tabs\nm move / dock · r resize with arrows · z maximize / restore\nw close view · u undo layout · a approvals · d diff · : command\nDrag tabs or sidebar rows onto pane edges or center (tab group).\nDrag dividers to resize; drag sidebar rows to reorder.\nPgUp/PgDn or wheel scroll; PgDn returns toward live output.\n\nCOMMANDS\n/side [question]  /btw [question]   open side on the right\n/side-panel [show|hide]  /find [text]  /live  /tasks\n/new [directory]  /search  /quick-switch  /archived  /rename [title]  /pin\n/model [id]  /models  /cwd [directory]  /project [name]\n/permissions read-only|workspace-write  /worktree path ref\n/queue [clear|release]  /steer message  /stop  /history\n/send-parent [text]  /parent  /promote  /move  /close\n/diff  /approvals  /archive  /restore  /refresh\n/login  /reconnect  /quit  /help\nPrefix a literal slash message with // . Esc cancels dialogs.";

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}
pub fn run(client: Client) -> Result<()> {
    run_app(App::new(Some(client)))
}
pub fn run_demo() -> Result<()> {
    run_app(demo())
}
fn run_app(mut app: App) -> Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal(),
        "Run Codex Deck in an interactive terminal; use --help for commands."
    );
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        previous(info)
    }));
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut redraw = true;
    let mut toast_visible = false;
    while !app.quit {
        redraw |= app.receive();
        if app.last_save.elapsed() > Duration::from_millis(400) {
            app.save_drafts();
        }
        if app.last_read.elapsed() > Duration::from_secs(2)
            && app
                .snap
                .data
                .conversations
                .get(&app.focus)
                .is_some_and(|c| c.unread)
        {
            app.fire("read", json!({"thread":app.focus}));
            app.last_read = Instant::now();
        }
        let visible = !app.toast.is_empty() && app.toast_at.elapsed() < Duration::from_secs(10);
        redraw |= visible != toast_visible;
        toast_visible = visible;
        if redraw {
            terminal.draw(|f| app.draw(f))?;
            redraw = false;
        }
        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        let batch = Instant::now();
        loop {
            redraw = true;
            match event::read()? {
                Event::Key(k) => app.key(k),
                Event::Mouse(m) => app.mouse(m),
                Event::Paste(s) => {
                    if let Some(modal) = &mut app.modal {
                        match modal {
                            Modal::Find {
                                query, selected, ..
                            }
                            | Modal::Tasks {
                                query, selected, ..
                            } => {
                                query.push_str(&s.replace(['\r', '\n'], " "));
                                *selected = 0;
                            }
                            Modal::Models {
                                query, selected, ..
                            } => {
                                query.push_str(&s.replace(['\r', '\n'], " "));
                                *selected = 0;
                            }
                            Modal::New { fields, focus, .. } => fields[*focus].push_str(&s),
                            Modal::Prompt { value, .. } => value.push_str(&s),
                            Modal::Pending { input, .. } => input.push_str(&s),
                            _ => {}
                        }
                    } else if app.manager_focused {
                        app.manager_query.push_str(&s.replace(['\r', '\n'], " "));
                        app.manager_filtering = true;
                        app.manager_query_changed();
                    } else {
                        app.insert(&s)
                    }
                }
                _ => {}
            }
            if app.quit
                || batch.elapsed() >= Duration::from_millis(4)
                || !event::poll(Duration::ZERO)?
            {
                break;
            }
        }
    }
    app.save_drafts();
    app.persist();
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}
fn demo() -> App {
    let mut app = App::new(None);
    app.demo = true;
    app.booted = true;
    app.snap.connected = true;
    app.root = "demo-main".into();
    app.focus = "demo-main".into();
    app.layout = Node::single("demo-main");
    app.layout.dock("demo-side", "demo-main", "right");
    for (id, title, cwd, status) in [
        (
            "demo-main",
            "Implement login",
            "examples/platform",
            "running",
        ),
        (
            "demo-side",
            "Side · failure cases",
            "examples/platform",
            "completed",
        ),
        (
            "demo-api",
            "API migration",
            "examples/api",
            "waitingApproval",
        ),
        (
            "demo-docs",
            "Update documentation",
            "examples/docs",
            "completed",
        ),
    ] {
        let mut c = Conversation::new(id.into(), title.into(), cwd.into());
        c.status = status.into();
        c.model = "gpt-5.4".into();
        c.project = Some(
            if id == "demo-docs" {
                "Documentation"
            } else {
                "Platform"
            }
            .into(),
        );
        if id == "demo-side" {
            c.parent = Some("demo-main".into());
            c.readonly = true;
        }
        app.snap.data.conversations.insert(id.into(), c);
    }
    app.snap.data.projects = vec![
        Project {
            id: "p1".into(),
            name: "Platform".into(),
            roots: vec![],
        },
        Project {
            id: "p2".into(),
            name: "Documentation".into(),
            roots: vec![],
        },
    ];
    let c = app.snap.data.conversations.get_mut("demo-main").unwrap();
    c.messages=vec![Message{id:"u".into(),role:"You".into(),text:"实现登录流程，保留现有 API，并补充错误处理。".into(),status:String::new()},Message{id:"a".into(),role:"Codex".into(),text:"我正在检查会话验证和错误路径。\n\n已经完成：\n• 验证 token 生命周期\n• 为登录表单加入清晰的错误提示\n\n接下来运行测试，并检查回归。".into(),status:String::new()},Message{id:"cmd".into(),role:"Command".into(),text:"$ cargo test auth\n\nrunning 12 tests\ntest auth::refresh_token ... ok".into(),status:"inProgress".into()}];
    app.snap.data.conversations.get_mut("demo-side").unwrap().messages=vec![Message{id:"s1".into(),role:"You".into(),text:"检查登录失败时的边界情况。".into(),status:String::new()},Message{id:"s2".into(),role:"Codex".into(),text:"建议覆盖三种情况：\n\n1. 过期 token 的刷新失败\n2. 网络中断后再次提交\n3. 用户取消授权\n\n使用 /send-parent 把这些建议放入主对话草稿。".into(),status:String::new()}];
    app.drafts.insert(
        "demo-main".into(),
        Draft {
            text: "继续检查并发登录的情况".into(),
            cursor: "继续检查并发登录的情况".len(),
        },
    );
    app
}
pub fn render_demo() -> Result<()> {
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 40))?;
    let mut app = demo();
    terminal.draw(|f| app.draw(f))?;
    let b = terminal.backend().buffer();
    for y in 0..40 {
        let mut line = String::new();
        let mut x = 0;
        while x < 160 {
            let symbol = b[(x, y)].symbol();
            line.push_str(symbol);
            x += symbol.width().max(1) as u16;
        }
        println!("{}", line.trim_end());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn draw_at(app: &mut App, width: u16) {
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(width, 32)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
    }
    #[test]
    fn startup_hides_saved_side_views_without_discarding_them() {
        let saved = demo();
        let layout = saved.layout.clone();
        let mut snapshot = saved.snap.clone();
        snapshot
            .data
            .layouts
            .insert("demo-main".into(), layout.clone());
        snapshot.data.active = Some("demo-main".into());
        snapshot.data.sidebar = true;
        let (client, input, _) = Client::fixture();
        let mut app = App::new(Some(client));
        let empty = Snapshot {
            connected: true,
            ..Default::default()
        };
        input
            .send(json!({"type":"snapshot","snapshot":empty}))
            .unwrap();
        app.receive();
        assert!(!app.booted);
        input
            .send(json!({"type":"snapshot","snapshot":snapshot}))
            .unwrap();
        app.receive();
        for width in [80, 100, 160] {
            draw_at(&mut app, width);
            assert_eq!(app.panes.len(), 1);
        }
        assert_eq!(app.layout, layout);
        assert!(app.snap.data.conversations.contains_key("demo-side"));
        assert_eq!(app.snap.data.conversations["demo-main"].status, "running");
        app.open("demo-side");
        draw_at(&mut app, 100);
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.focus, "demo-side");
    }
    #[test]
    fn collapsed_side_restores_layout_and_last_close_reclaims_space() {
        let mut app = demo();
        let saved = app.layout.clone();
        app.toggle_side(Some(false));
        draw_at(&mut app, 100);
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.focus, "demo-main");
        assert_eq!(app.layout, saved);
        app.open("demo-side");
        draw_at(&mut app, 100);
        assert_eq!(app.panes.len(), 2);
        app.close();
        draw_at(&mut app, 100);
        assert_eq!(app.panes.len(), 1);
        assert!(app.snap.data.conversations.contains_key("demo-side"));
        assert_eq!(app.snap.data.conversations["demo-main"].status, "running");
        app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        draw_at(&mut app, 100);
        assert_eq!(app.panes.len(), 2);
        app.toggle_side(Some(false));
        app.toggle_side(Some(true));
        draw_at(&mut app, 80);
        assert_eq!(app.panes.len(), 2);
    }
    #[test]
    fn task_overview_filters_attention_and_does_not_stop_jobs() {
        let mut app = demo();
        let (client, _, mut out) = Client::fixture();
        app.client = Some(client);
        let draft = app.drafts["demo-main"].text.clone();
        app.command("/tasks");
        for w in [80, 100, 160] {
            draw_at(&mut app, w);
        }
        assert!(app.task_rows("", 1).contains(&"demo-main".into()));
        assert!(app.task_rows("", 3).contains(&"demo-api".into()));
        app.modal = Some(Modal::Tasks {
            query: "API migration".into(),
            selected: 0,
            filter: 3,
        });
        app.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus, "demo-api");
        assert_eq!(app.drafts["demo-main"].text, draft);
        assert!(!std::iter::from_fn(|| out.try_recv().ok())
            .any(|v| v["op"] == "stop" || v["op"] == "send"));
    }
    #[test]
    fn model_picker_targets_its_conversation_and_uses_backend_model_value() {
        let (client, _, mut outgoing) = Client::fixture();
        let mut app = demo();
        app.client = Some(client);
        app.snap.models = vec![
            json!({"id":"catalog-a","model":"model-a","displayName":"Model A","isDefault":true}),
            json!({"id":"catalog-b","model":"model-b","displayName":"Model B"}),
        ];
        app.focus("demo-side");
        app.snap
            .data
            .conversations
            .get_mut("demo-side")
            .unwrap()
            .model = "model-a".into();
        app.command("/model");
        // A model dialog belongs to its original conversation, even if focus changes.
        app.focus = "demo-main".into();
        app.modal_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let settings = std::iter::from_fn(|| outgoing.try_recv().ok())
            .find(|v| v["op"] == "settings")
            .unwrap();
        assert_eq!(
            settings["args"],
            json!({"thread":"demo-side","model":"model-b"})
        );
        assert!(app.modal.is_none());
        app.command("/model");
        app.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.modal.is_some(), "running tasks must not change models");
        assert!(outgoing.try_recv().is_err());
        app.modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.snap.models.clear();
        app.command("/models");
        for width in [80, 100, 160] {
            draw_at(&mut app, width);
        }
        app.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            outgoing.try_recv().is_err(),
            "an empty list must never apply an empty model"
        );
    }
    #[test]
    fn manager_main_and_side_stay_visible_at_normal_terminal_sizes() {
        let mut app = demo();
        for width in [80, 100, 160] {
            for focus in ["demo-main", "demo-side"] {
                app.focus(focus);
                draw_at(&mut app, width);
                let sidebar = app.sidebar_rect.unwrap();
                assert_eq!(app.panes.len(), 2, "width={width}");
                assert!(app.panes[0].rect.x > sidebar.right());
                assert!(app.panes[1].rect.x > app.panes[0].rect.right());
                assert_eq!(app.panes[0].ids, ["demo-main"]);
                assert_eq!(app.panes[1].ids, ["demo-side"]);
                assert!(app.modal.is_none());
            }
        }
        app.layout = Node::single("demo-main");
        draw_at(&mut app, 80);
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.layout, Node::single("demo-main"));
    }
    #[test]
    fn directories_group_chats_without_merging_equal_folder_names() {
        let mut app = demo();
        app.snap
            .data
            .conversations
            .get_mut("demo-main")
            .unwrap()
            .cwd = "examples/one/api/".into();
        app.snap
            .data
            .conversations
            .get_mut("demo-side")
            .unwrap()
            .cwd = "examples/one/api".into();
        app.snap.data.conversations.get_mut("demo-api").unwrap().cwd = "examples/two/api".into();
        let rows = app.rows();
        let groups: Vec<_> = rows
            .iter()
            .filter_map(|row| match &row.target {
                Some(ManagerTarget::Directory(path)) => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert!(groups.contains(&"examples/one/api") && groups.contains(&"examples/two/api"));
        let children: Vec<_> = rows
            .iter()
            .filter_map(|row| match &row.target {
                Some(ManagerTarget::Conversation(key)) => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            children.iter().filter(|key| **key == "demo-side").count(),
            1
        );
        app.toggle_directory("examples/one/api");
        assert!(!app
            .rows()
            .iter()
            .any(|row| row.target == Some(ManagerTarget::Conversation("demo-main".into()))));
        assert!(app
            .rows()
            .iter()
            .any(|row| row.target == Some(ManagerTarget::Conversation("demo-api".into()))));
    }
    #[test]
    fn inline_filter_and_mouse_switch_without_touching_chat_drafts() {
        let mut app = demo();
        let draft = app.drafts["demo-main"].text.clone();
        app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(app.manager_focused && app.modal.is_none());
        for c in "demo-docs".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.drafts["demo-main"].text, draft);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus, "demo-docs");
        assert!(app.modal.is_none() && !app.manager_focused);
        app.manager_query.clear();
        draw_at(&mut app, 100);
        let row = app
            .hits
            .iter()
            .find_map(|(r, hit)| matches!(hit,Hit::Open(key) if key=="demo-side").then_some(*r))
            .unwrap();
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            app.mouse(MouseEvent {
                kind,
                column: row.x,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            });
        }
        assert_eq!(app.focus, "demo-side");
        assert_eq!(app.root, "demo-main");
        draw_at(&mut app, 80);
        assert_eq!(app.panes[0].ids, ["demo-main"]);
        assert!(app.panes[1].ids.contains(&"demo-side".into()));
        assert_eq!(app.drafts["demo-main"].text, draft);
    }
    #[test]
    fn nested_sides_share_the_right_tab_group() {
        let (client, input, mut outgoing) = Client::fixture();
        let mut app = demo();
        app.client = Some(client);
        app.focus("demo-side");
        app.command("/btw nested");
        let request = std::iter::from_fn(|| outgoing.try_recv().ok())
            .find(|v| v["op"] == "side")
            .unwrap();
        input.send(json!({"type":"response","id":request["id"],"result":{"thread":"nested","parent":"demo-side"}})).unwrap();
        app.receive();
        draw_at(&mut app, 80);
        assert_eq!(app.root, "demo-main");
        assert_eq!(app.layout.leaves(), 2);
        assert_eq!(app.panes[0].ids, ["demo-main"]);
        assert!(
            app.panes[1].ids.contains(&"demo-side".into())
                && app.panes[1].ids.contains(&"nested".into())
        );
    }
    #[test]
    fn cached_viewport_matches_full_paragraph_wrapping() {
        let mut c = Conversation::new(
            "test".into(),
            "Test".into(),
            std::env::temp_dir().to_string_lossy().into_owned(),
        );
        c.messages = vec![Message {
            id: "message".into(),
            role: "Codex".into(),
            status: String::new(),
            text: "word wrap with indent\n    preserved spaces and 中文 👩‍💻 é\n\n".repeat(30),
        }];
        for width in [12, 40, 90] {
            let mut cache = Transcript::default();
            cache.update(&c, None, width);
            let mut lines = vec![Line::styled(
                "Codex",
                style(FG).add_modifier(Modifier::BOLD),
            )];
            lines.extend(
                c.messages[0]
                    .text
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), style(FG))),
            );
            lines.push(Line::default());
            let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
            assert_eq!(cache.height, paragraph.line_count(width));
            for skip in [0, 1, 4, cache.height.saturating_sub(11)] {
                let mut actual =
                    Terminal::new(ratatui::backend::TestBackend::new(width, 11)).unwrap();
                let mut expected =
                    Terminal::new(ratatui::backend::TestBackend::new(width, 11)).unwrap();
                actual.draw(|f| cache.render(f, f.area(), skip)).unwrap();
                expected
                    .draw(|f| f.render_widget(paragraph.clone().scroll((skip as u16, 0)), f.area()))
                    .unwrap();
                assert_eq!(
                    actual.backend().buffer(),
                    expected.backend().buffer(),
                    "width={width} skip={skip}"
                );
            }
        }
    }

    #[test]
    fn history_height_does_not_wrap_at_u16_limit() {
        let mut c = Conversation::new(
            "test".into(),
            "Test".into(),
            std::env::temp_dir().to_string_lossy().into_owned(),
        );
        c.messages.push(Message {
            id: "long".into(),
            role: "Codex".into(),
            text: "history\n".repeat(66000) + "latest marker",
            status: String::new(),
        });
        let mut cache = Transcript::default();
        cache.update(&c, None, 40);
        assert!(cache.height > u16::MAX as usize);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(40, 6)).unwrap();
        terminal
            .draw(|f| cache.render(f, f.area(), cache.height - 6))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("latest marker"));
    }
    #[test]
    #[ignore = "manual large-history rendering benchmark"]
    fn large_history_render_benchmark() {
        let mut app = demo();
        app.layout = Node::single("demo-main");
        app.snap
            .data
            .conversations
            .get_mut("demo-main")
            .unwrap()
            .messages = (0..1000)
            .map(|i| Message {
                id: format!("message-{i}"),
                role: "Codex".into(),
                text: "History text with words and 中文 👩‍💻. ".repeat(30),
                status: String::new(),
            })
            .collect();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
        let started = Instant::now();
        terminal.draw(|f| app.draw(f)).unwrap();
        println!(
            "initial_history_render_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        let started = Instant::now();
        for _ in 0..20 {
            app.insert("x");
            terminal.draw(|f| app.draw(f)).unwrap();
        }
        println!(
            "typing_frame_mean_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0 / 20.0
        );
    }
    #[test]
    fn renders_all_sizes_without_losing_layout() {
        let mut app = demo();
        let original = app.layout.clone();
        for (w, h) in [(160, 40), (100, 24), (80, 24), (40, 12), (12, 4)] {
            let mut t = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            t.draw(|f| app.draw(f)).unwrap();
            assert_eq!(original, app.layout);
        }
    }
    #[test]
    fn editing_respects_graphemes() {
        let mut app = demo();
        app.drafts.insert(app.focus.clone(), Draft::default());
        app.insert("你👩‍💻é");
        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.draft().text, "你👩‍💻");
        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.draft().text, "你");
    }
    #[test]
    fn aliases_open_identical_side_effects() {
        for command in ["/side explain", "/btw explain"] {
            let (client, input, mut outgoing) = Client::fixture();
            let mut a = demo();
            a.client = Some(client);
            a.layout = Node::single("demo-main");
            a.command(command);
            let request = outgoing.try_recv().unwrap();
            assert_eq!(request["op"], "side");
            assert_eq!(request["args"]["thread"], "demo-main");
            input.send(json!({"type":"response","id":request["id"],"result":{"thread":"new-side","parent":"demo-main"}})).unwrap();
            a.receive();
            assert_eq!(a.focus, "new-side");
            let Node::Split {
                vertical, second, ..
            } = &a.layout
            else {
                panic!("side did not open on the right")
            };
            assert!(!vertical);
            let mut right = vec![];
            second.ids(&mut right);
            assert_eq!(right, vec!["new-side"]);
            assert!(
                std::iter::from_fn(|| outgoing.try_recv().ok()).any(|v| v["op"] == "send"
                    && v["args"]["thread"] == "new-side"
                    && v["args"]["text"] == "explain")
            );
        }
    }
    #[test]
    fn duplicate_enter_is_not_sent_twice_and_drafts_stay_scoped() {
        let (client, input, mut outgoing) = Client::fixture();
        let mut a = demo();
        a.client = Some(client);
        let original = a.draft().text.clone();
        a.submit();
        a.submit();
        let request = outgoing.try_recv().unwrap();
        assert!(outgoing.try_recv().is_err());
        a.focus = "demo-side".into();
        a.insert("other draft");
        input
            .send(json!({"type":"response","id":request["id"],"result":{"thread":"demo-main"}}))
            .unwrap();
        a.receive();
        assert_eq!(a.drafts["demo-main"].text, "");
        assert_eq!(a.drafts["demo-side"].text, "other draft");
        assert!(!original.is_empty());
    }
    #[test]
    fn closed_root_reopens_and_maximize_does_not_replace_saved_layout() {
        let mut a = demo();
        let original = a.layout.clone();
        a.maximize = Some(original.clone());
        a.layout = Node::single("demo-main");
        a.persist();
        assert_eq!(a.layouts["demo-main"], original);
        a.maximize = None;
        a.layout = Node::single("demo-main");
        a.close();
        a.open("demo-main");
        let mut ids = vec![];
        a.layout.ids(&mut ids);
        assert!(ids.contains(&"demo-main".into()));
    }
    #[test]
    fn incoming_output_preserves_reading_position() {
        let mut a = demo();
        a.layout = Node::single("demo-main");
        a.snap
            .data
            .conversations
            .get_mut("demo-main")
            .unwrap()
            .messages = vec![Message {
            id: "long".into(),
            role: "Codex".into(),
            text: (0..100).map(|i| format!("line {i}\n")).collect(),
            status: String::new(),
        }];
        let mut t = Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
        t.draw(|f| a.draw(f)).unwrap();
        a.scroll.insert("demo-main".into(), 20);
        let old = a.heights["demo-main"].0;
        a.snap
            .data
            .conversations
            .get_mut("demo-main")
            .unwrap()
            .messages[0]
            .text
            .push_str("new one\nnew two\n");
        t.draw(|f| a.draw(f)).unwrap();
        assert_eq!(
            old - 20,
            a.heights["demo-main"].0 - a.scroll["demo-main"] as usize
        );
    }
}
