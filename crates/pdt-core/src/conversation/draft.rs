//! Make ticket from a chat: what PDT asks the agent in its hidden side request, the
//! answer's shape, and reading the answer back. Pure.

use crate::ticket::{MAX_DESCRIPTION, MAX_TITLE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// The side request's only message: the agent sees the whole conversation before it.
pub const DRAFT_PROMPT: &str = "Draft a ticket for the work discussed in this conversation so far. Reply with only \
the JSON object the schema describes: a short imperative title (under 80 characters) and a Markdown description \
covering the goal, what is known or decided, and acceptance criteria as a checklist. Do not use tools and do not do \
the work.";

/// The JSON Schema of the answer (Claude's `--json-schema`, Codex's `outputSchema`).
pub fn draft_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "Short imperative ticket title"},
            "description": {"type": "string", "description": "Markdown ticket description"}
        },
        "required": ["title", "description"],
        "additionalProperties": false
    })
}

/// A drafted ticket, ready to prefill the new-ticket form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketDraft {
    pub title: String,
    pub description: String,
}

fn cut(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

impl TicketDraft {
    /// The draft in an answer: an object, or text holding one (Codex answers with text; a Markdown code fence around
    /// it is fine). Trimmed and cut to the ticket limits; None without a title.
    pub fn from_value(v: &Value) -> Option<Self> {
        let obj = match v {
            Value::Object(_) => v.clone(),
            Value::String(s) => serde_json::from_str::<Value>(unfence(s)).ok()?,
            _ => return None,
        };
        let title = obj.get("title")?.as_str()?.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return None;
        }
        let description = obj.get("description").and_then(Value::as_str).unwrap_or("").trim();
        let description = if description.len() > MAX_DESCRIPTION {
            let mut end = MAX_DESCRIPTION;
            while !description.is_char_boundary(end) {
                end -= 1;
            }
            &description[..end]
        } else {
            description
        };
        Some(Self { title: cut(&title, MAX_TITLE), description: description.to_string() })
    }
}

/// The text inside a ```` ``` ```` fence, or all of it.
fn unfence(s: &str) -> &str {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else { return t };
    let rest = rest.split_once('\n').map(|(_, r)| r).unwrap_or("");
    rest.trim_end().strip_suffix("```").unwrap_or(rest).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_object_or_text_holding_one() {
        let want = TicketDraft { title: "Fix login on Safari".into(), description: "## Goal\nIt works.".into() };
        let v = json!({"title": "  Fix login\n on Safari ", "description": "\n## Goal\nIt works.\n"});
        assert_eq!(TicketDraft::from_value(&v), Some(want.clone()));
        let text = Value::String(r###"{"title":"Fix login on Safari","description":"## Goal\nIt works."}"###.into());
        assert_eq!(TicketDraft::from_value(&text), Some(want.clone()));
        let fenced = Value::String(
            "```json\n{\"title\":\"Fix login on Safari\",\"description\":\"## Goal\\nIt works.\"}\n```".into(),
        );
        assert_eq!(TicketDraft::from_value(&fenced), Some(want));
    }

    #[test]
    fn a_draft_needs_a_title_and_fits_the_limits() {
        assert_eq!(TicketDraft::from_value(&json!({"title": " ", "description": "x"})), None);
        assert_eq!(TicketDraft::from_value(&json!({"description": "x"})), None);
        assert_eq!(TicketDraft::from_value(&Value::String("not json".into())), None);
        assert_eq!(TicketDraft::from_value(&json!(3)), None);
        let d = TicketDraft::from_value(&json!({"title": "t"})).unwrap();
        assert_eq!(d.description, "");
        let long =
            TicketDraft::from_value(&json!({"title": "é".repeat(500), "description": "ü".repeat(60_000)})).unwrap();
        assert_eq!(long.title.chars().count(), MAX_TITLE);
        assert!(long.title.ends_with('…'));
        assert!(long.description.len() <= MAX_DESCRIPTION);
    }

    #[test]
    fn the_schema_asks_for_both_fields() {
        let s = draft_schema();
        assert_eq!(s["required"], json!(["title", "description"]));
        assert_eq!(s["additionalProperties"], json!(false));
    }
}
