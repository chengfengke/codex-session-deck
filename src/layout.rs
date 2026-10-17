use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Node {
    Tabs {
        ids: Vec<String>,
        active: usize,
    },
    Split {
        vertical: bool,
        ratio: u16,
        first: Box<Node>,
        second: Box<Node>,
    },
}
#[derive(Clone, Debug)]
pub struct Pane {
    pub path: Vec<bool>,
    pub rect: Rect,
    pub ids: Vec<String>,
    pub active: usize,
}
#[derive(Clone, Debug)]
pub struct Divider {
    pub path: Vec<bool>,
    pub rect: Rect,
    pub whole: Rect,
    pub vertical: bool,
}
impl Node {
    pub fn single(id: &str) -> Self {
        Self::Tabs {
            ids: vec![id.into()],
            active: 0,
        }
    }
    pub fn replace(&mut self, old: &str, new: &str) {
        match self {
            Self::Tabs { ids, .. } => {
                for id in ids {
                    if id == old {
                        *id = new.into()
                    }
                }
            }
            Self::Split { first, second, .. } => {
                first.replace(old, new);
                second.replace(old, new);
            }
        }
    }
    pub fn panes(&self, area: Rect) -> (Vec<Pane>, Vec<Divider>) {
        let mut p = vec![];
        let mut d = vec![];
        self.walk(area, vec![], &mut p, &mut d);
        (p, d)
    }
    fn walk(&self, r: Rect, path: Vec<bool>, p: &mut Vec<Pane>, d: &mut Vec<Divider>) {
        match self {
            Self::Tabs { ids, active } => p.push(Pane {
                path,
                rect: r,
                ids: ids.clone(),
                active: (*active).min(ids.len().saturating_sub(1)),
            }),
            Self::Split {
                vertical,
                ratio,
                first,
                second,
            } => {
                let size = if *vertical { r.height } else { r.width };
                let min = if *vertical { 12 } else { 20 };
                if size < min * 2 + 1 {
                    let mut ids = vec![];
                    self.ids(&mut ids);
                    p.push(Pane {
                        path,
                        rect: r,
                        ids,
                        active: 0,
                    });
                    return;
                }
                let n = ((size.saturating_sub(1) as u32 * (*ratio as u32)) / 1000) as u16;
                let n = n.clamp(min, size - min - 1);
                let (a, line, b) = if *vertical {
                    (
                        Rect::new(r.x, r.y, r.width, n),
                        Rect::new(r.x, r.y + n, r.width, 1),
                        Rect::new(r.x, r.y + n + 1, r.width, size - n - 1),
                    )
                } else {
                    (
                        Rect::new(r.x, r.y, n, r.height),
                        Rect::new(r.x + n, r.y, 1, r.height),
                        Rect::new(r.x + n + 1, r.y, size - n - 1, r.height),
                    )
                };
                d.push(Divider {
                    path: path.clone(),
                    rect: line,
                    whole: r,
                    vertical: *vertical,
                });
                let mut ap = path.clone();
                ap.push(false);
                let mut bp = path;
                bp.push(true);
                first.walk(a, ap, p, d);
                second.walk(b, bp, p, d);
            }
        }
    }
    pub fn ids(&self, out: &mut Vec<String>) {
        match self {
            Self::Tabs { ids, .. } => out.extend(ids.clone()),
            Self::Split { first, second, .. } => {
                first.ids(out);
                second.ids(out);
            }
        }
    }
    pub fn active_ids(&self, out: &mut Vec<String>) {
        match self {
            Self::Tabs { ids, active } => {
                if let Some(id) = ids.get((*active).min(ids.len().saturating_sub(1))) {
                    out.push(id.clone());
                }
            }
            Self::Split { first, second, .. } => {
                first.active_ids(out);
                second.active_ids(out);
            }
        }
    }
    pub fn at_mut(&mut self, path: &[bool]) -> Option<&mut Node> {
        if path.is_empty() {
            return Some(self);
        }
        match self {
            Self::Split { first, second, .. } => {
                if path[0] {
                    second.at_mut(&path[1..])
                } else {
                    first.at_mut(&path[1..])
                }
            }
            _ => None,
        }
    }
    pub fn focus(&mut self, id: &str) -> bool {
        match self {
            Self::Tabs { ids, active } => {
                if let Some(i) = ids.iter().position(|x| x == id) {
                    *active = i;
                    true
                } else {
                    false
                }
            }
            Self::Split { first, second, .. } => first.focus(id) || second.focus(id),
        }
    }
    pub fn remove(&mut self, id: &str) {
        match self {
            Self::Tabs { ids, active } => {
                ids.retain(|x| x != id);
                *active = (*active).min(ids.len().saturating_sub(1));
            }
            Self::Split { first, second, .. } => {
                first.remove(id);
                second.remove(id);
                let mut a = vec![];
                first.ids(&mut a);
                let mut b = vec![];
                second.ids(&mut b);
                if a.is_empty() {
                    *self = (**second).clone()
                } else if b.is_empty() {
                    *self = (**first).clone()
                }
            }
        }
    }
    pub fn dock(&mut self, id: &str, target: &str, edge: &str) -> bool {
        if id == target {
            return false;
        }
        let mut candidate = self.clone();
        candidate.remove(id);
        if !candidate.insert(id, target, edge) {
            return false;
        }
        *self = candidate;
        true
    }
    fn insert(&mut self, id: &str, target: &str, edge: &str) -> bool {
        match self {
            Self::Tabs { ids, active } => {
                if !ids.iter().any(|x| x == target) {
                    return false;
                }
                if edge == "center" {
                    let i = ids.iter().position(|x| x == target).unwrap_or(ids.len());
                    ids.insert(i, id.into());
                    *active = i;
                    return true;
                }
                let old = self.clone();
                let fresh = Self::single(id);
                let before = matches!(edge, "left" | "up");
                *self = Self::Split {
                    vertical: matches!(edge, "up" | "down"),
                    ratio: if before { 400 } else { 600 },
                    first: Box::new(if before { fresh.clone() } else { old.clone() }),
                    second: Box::new(if before { old } else { fresh }),
                };
                true
            }
            Self::Split { first, second, .. } => {
                first.insert(id, target, edge) || second.insert(id, target, edge)
            }
        }
    }
    pub fn leaves(&self) -> usize {
        match self {
            Self::Tabs { .. } => 1,
            Self::Split { first, second, .. } => first.leaves() + second.leaves(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dock_preserves_identity_and_rejects_missing_target() {
        let mut n = Node::single("a");
        assert!(n.dock("b", "a", "right"));
        assert!(n.dock("c", "b", "down"));
        let saved = n.clone();
        assert!(!n.dock("a", "missing", "left"));
        assert_eq!(n, saved);
        assert!(n.dock("b", "c", "center"));
        let mut ids = vec![];
        n.ids(&mut ids);
        ids.sort();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
    #[test]
    fn narrow_layout_collapses_without_destroying_tree() {
        let mut n = Node::single("a");
        n.dock("b", "a", "right");
        let original = n.clone();
        let (p, _) = n.panes(Rect::new(0, 0, 38, 24));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].ids.len(), 2);
        assert_eq!(n, original);
        assert_eq!(n.panes(Rect::new(0, 0, 130, 30)).0.len(), 2);
    }
}
