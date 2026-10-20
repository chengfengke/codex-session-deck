use crate::model::Conversation;
use serde_json::{json, Value};
use std::collections::HashMap;

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}

pub fn role(kind: &str) -> String {
    match kind {
        "userMessage" => "You",
        "agentMessage" => "Codex",
        "commandExecution" => "Command",
        "fileChange" => "Files",
        "reasoning" => "Thinking",
        "webSearch" => "Search",
        "collabAgentToolCall" => "Agents",
        "contextCompaction" => "Context",
        _ => "Tool",
    }
    .into()
}

fn content(v: &Value) -> String {
    if let Some(s) = v.as_str().or_else(|| v["text"].as_str()) {
        s.into()
    } else if let Some(a) = v.as_array() {
        a.iter()
            .map(content)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    }
}

pub fn item_text(v: &Value) -> String {
    let kind = v["type"].as_str().unwrap_or_default();
    match kind {
        "reasoning" => return content(&v["summary"]),
        "commandExecution" => {
            let mut lines = vec![format!("$ {}", text(v, "command"))];
            if let Some(cwd) = v["cwd"].as_str().filter(|s| !s.is_empty()) {
                lines.push(format!("cwd: {cwd}"));
            }
            let output = text(v, "aggregatedOutput");
            if !output.is_empty() {
                lines.push(output);
            }
            if let Some(code) = v["exitCode"].as_i64() {
                lines.push(format!("exit: {code}"));
            }
            if let Some(ms) = v["durationMs"].as_i64() {
                lines.push(format!("duration: {ms} ms"));
            }
            return lines.join("\n");
        }
        "fileChange" => {
            let mut lines = vec![];
            if let Some(changes) = v["changes"].as_array() {
                for change in changes {
                    let kind = change["kind"]["type"]
                        .as_str()
                        .or_else(|| change["kind"].as_str())
                        .unwrap_or("update");
                    lines.push(format!("{kind} {}", text(change, "path")));
                    if let Some(diff) = change["diff"].as_str() {
                        lines.push(diff.into());
                    }
                }
            }
            if let Some(output) = v["aggregatedOutput"].as_str() {
                lines.push(output.into());
            }
            return lines.join("\n");
        }
        "mcpToolCall" | "dynamicToolCall" => {
            let server = text(v, "server");
            let tool = text(v, "tool");
            let mut lines = vec![if server.is_empty() {
                tool
            } else {
                format!("{server} · {tool}")
            }];
            if let Some(args) = v.get("arguments").filter(|a| !a.is_null()) {
                lines.push(format!("input: {args}"));
            }
            let progress = content(&v["progress"]);
            if !progress.is_empty() {
                lines.push(progress);
            }
            let result = content(&v["result"]["content"]);
            if !result.is_empty() {
                lines.push(result);
            } else if let Some(result) = v["result"]
                .get("structuredContent")
                .filter(|a| !a.is_null())
            {
                lines.push(result.to_string());
            }
            let dynamic = content(&v["contentItems"]);
            if !dynamic.is_empty() {
                lines.push(dynamic);
            }
            if let Some(error) = v.get("error").filter(|a| !a.is_null()) {
                lines.push(format!(
                    "error: {}",
                    error["message"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| error.to_string())
                ));
            }
            if let Some(ms) = v["durationMs"].as_i64() {
                lines.push(format!("duration: {ms} ms"));
            }
            return lines.join("\n");
        }
        "webSearch" => return format!("Search: {}", text(v, "query")),
        "collabAgentToolCall" => {
            let mut lines = vec![text(v, "tool")];
            if let Some(states) = v["agentsStates"].as_object() {
                for (id, state) in states {
                    lines.push(format!("{id}: {}", text(state, "status")));
                }
            }
            return lines.join("\n");
        }
        "contextCompaction" => return "Conversation context compacted".into(),
        _ => {}
    }
    if let Some(s) = v["text"].as_str() {
        return s.into();
    }
    if v["content"].is_array() {
        return content(&v["content"]);
    }
    if let Some(s) = v["output"].as_str() {
        return s.into();
    }
    format!(
        "{} {}",
        text(v, "type"),
        v.get("tool").unwrap_or(&Value::Null)
    )
}

/// Retain only public display fields while a turn streams. Raw reasoning is not displayed.
#[derive(Default)]
pub struct Activity {
    items: HashMap<(String, String), Value>,
}

impl Activity {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn finish(&mut self, c: &mut Conversation, turn: &str, status: &str) {
        for ((thread, id), item) in &self.items {
            if thread == &c.id && item["_turn"] == turn {
                if let Some(m) = c
                    .messages
                    .iter_mut()
                    .find(|m| &m.id == id && m.status == "inProgress")
                {
                    m.status = status.into();
                }
            }
        }
        self.items
            .retain(|(thread, _), item| thread != &c.id || item["_turn"] != turn);
        if let Some(plan) = c
            .messages
            .iter_mut()
            .find(|m| m.id == format!("plan:{turn}") && m.status == "inProgress")
        {
            plan.status = status.into();
        }
    }

    pub fn update(&mut self, c: &mut Conversation, method: &str, p: &Value) -> bool {
        if method == "turn/plan/updated" {
            let key = format!("plan:{}", text(p, "turnId"));
            let m = c.item(&key, "Plan");
            let mut lines = vec![];
            if let Some(s) = p["explanation"].as_str().filter(|s| !s.is_empty()) {
                lines.push(s.into());
            }
            let steps = p["plan"].as_array().map(Vec::as_slice).unwrap_or_default();
            for step in steps {
                let marker = match step["status"].as_str().unwrap_or_default() {
                    "completed" => "[x]",
                    "in_progress" => "[>]",
                    _ => "[ ]",
                };
                lines.push(format!("{marker} {}", text(step, "step")));
            }
            m.text = lines.join("\n");
            m.status = if !steps.is_empty() && steps.iter().all(|s| s["status"] == "completed") {
                "completed"
            } else {
                "inProgress"
            }
            .into();
            return true;
        }
        let (id, kind) = match method {
            "item/started" | "item/completed" => (text(&p["item"], "id"), text(&p["item"], "type")),
            "item/agentMessage/delta" => (text(p, "itemId"), "agentMessage".into()),
            "item/reasoning/summaryTextDelta" | "item/reasoning/summaryPartAdded" => {
                (text(p, "itemId"), "reasoning".into())
            }
            "item/commandExecution/outputDelta" => (text(p, "itemId"), "commandExecution".into()),
            "item/fileChange/outputDelta" => (text(p, "itemId"), "fileChange".into()),
            "item/mcpToolCall/progress" => (text(p, "itemId"), "mcpToolCall".into()),
            // The public summary channel is sufficient for the transcript.
            "item/reasoning/textDelta" => return true,
            _ => return false,
        };
        if id.is_empty() {
            return true;
        }
        let item = self.items.entry((c.id.clone(), id.clone())).or_insert_with(
            || json!({"id":id,"type":kind,"_turn":p["turnId"],"status":"inProgress"}),
        );
        if method == "item/started" || method == "item/completed" {
            if let Some(fields) = p["item"].as_object() {
                for (key, value) in fields {
                    if kind == "reasoning" && matches!(key.as_str(), "content" | "encryptedContent")
                    {
                        continue;
                    }
                    // Several backends omit the final aggregate after streaming it.
                    if value.is_null()
                        || (matches!(key.as_str(), "summary" | "aggregatedOutput" | "text")
                            && (value.as_str().is_some_and(str::is_empty)
                                || value.as_array().is_some_and(Vec::is_empty))
                            && item.get(key).is_some())
                    {
                        continue;
                    }
                    item[key] = value.clone();
                }
            }
            item["status"] = p["item"]["status"]
                .as_str()
                .unwrap_or(if method == "item/completed" {
                    "completed"
                } else {
                    "inProgress"
                })
                .into();
        } else if method.starts_with("item/reasoning/") {
            let index = p["summaryIndex"].as_u64().unwrap_or_default() as usize;
            if index > 1024 {
                return true;
            }
            if !item["summary"].is_array() {
                item["summary"] = json!([]);
            }
            let parts = item["summary"].as_array_mut().unwrap();
            parts.resize(parts.len().max(index + 1), json!(""));
            if method.ends_with("summaryTextDelta") {
                let old = content(&parts[index]);
                parts[index] = json!(old + p["delta"].as_str().unwrap_or_default());
            }
        } else if method == "item/mcpToolCall/progress" {
            if !item["progress"].is_array() {
                item["progress"] = json!([]);
            }
            item["progress"]
                .as_array_mut()
                .unwrap()
                .push(p["message"].clone());
        } else {
            let field = if kind == "agentMessage" {
                "text"
            } else {
                "aggregatedOutput"
            };
            let old = text(item, field);
            item[field] = json!(old + p["delta"].as_str().unwrap_or_default());
        }
        let m = c.item(&id, &role(&kind));
        m.role = role(&kind);
        m.text = item_text(item);
        m.status = text(item, "status");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_summary_parts_and_preserves_output_when_completion_has_no_aggregate() {
        let mut activity = Activity::default();
        let mut c = Conversation::new("a".into(), "Test".into(), ".".into());
        let event =
            |id: &str, delta: &str| json!({"threadId":"a","turnId":"t","itemId":id,"delta":delta});
        activity.update(&mut c, "item/started", &json!({"turnId":"t","item":{"id":"r","type":"reasoning","summary":[],"content":["PRIVATE_RAW"]}}));
        activity.update(
            &mut c,
            "item/reasoning/summaryTextDelta",
            &json!({"turnId":"t","itemId":"r","summaryIndex":1,"delta":"second"}),
        );
        activity.update(
            &mut c,
            "item/reasoning/summaryPartAdded",
            &json!({"turnId":"t","itemId":"r","summaryIndex":1}),
        );
        activity.update(
            &mut c,
            "item/reasoning/summaryTextDelta",
            &json!({"turnId":"t","itemId":"r","summaryIndex":0,"delta":"first"}),
        );
        activity.update(
            &mut c,
            "item/reasoning/textDelta",
            &event("r", "PRIVATE_RAW"),
        );
        activity.update(
            &mut c,
            "item/completed",
            &json!({"turnId":"t","item":{"id":"r","type":"reasoning","summary":[]}}),
        );
        assert_eq!(c.messages[0].text, "first\nsecond");
        assert_eq!(c.messages[0].status, "completed");
        activity.update(&mut c, "item/started", &json!({"turnId":"t","item":{"id":"cmd","type":"commandExecution","command":"echo ok","cwd":"workspace","status":"inProgress"}}));
        activity.update(
            &mut c,
            "item/commandExecution/outputDelta",
            &event("cmd", "ok\n"),
        );
        activity.update(&mut c, "item/completed", &json!({"turnId":"t","item":{"id":"cmd","type":"commandExecution","command":"echo ok","aggregatedOutput":null,"exitCode":0,"durationMs":25,"status":"completed"}}));
        assert!(c.messages[1].text.contains("ok\n"));
        assert!(c.messages[1].text.contains("exit: 0"));
        assert!(c.messages[1].text.contains("duration: 25 ms"));
        assert!(!c.messages.iter().any(|m| m.text.contains("PRIVATE_RAW")));
    }
    #[test]
    fn progress_is_scoped_to_its_conversation_and_turn() {
        let mut activity = Activity::default();
        let mut a = Conversation::new("a".into(), "A".into(), ".".into());
        let mut b = Conversation::new("b".into(), "B".into(), ".".into());
        for (c, delta) in [(&mut a, "ONLY_A"), (&mut b, "ONLY_B")] {
            activity.update(
                c,
                "item/reasoning/summaryTextDelta",
                &json!({"turnId":"t","itemId":"same","delta":delta,"summaryIndex":0}),
            );
        }
        activity.update(&mut a,"turn/plan/updated",&json!({"turnId":"t","plan":[{"step":"Read sources","status":"completed"},{"step":"Run tests","status":"in_progress"}]}));
        assert_eq!(a.messages[0].text, "ONLY_A");
        assert_eq!(b.messages[0].text, "ONLY_B");
        assert!(a.messages[1].text.contains("[>] Run tests"));
        activity.finish(&mut a, "t", "interrupted");
        assert_eq!(a.messages[0].status, "interrupted");
        assert_eq!(a.messages[1].status, "interrupted");
        assert_eq!(b.messages[0].status, "inProgress");
    }
}
