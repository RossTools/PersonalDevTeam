//! PDT's guides, built into pdtd so they are there on any host, with or without a copy of PDT's repository (PDT-46).
//! The sources are the Markdown files in docs/; agents read them through `GET /api/guides/{name}`.

use crate::error::ApiError;
use axum::Json;
use axum::extract::Path;
use axum::http::header;
use axum::response::IntoResponse;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GuideInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
}

struct Guide {
    info: GuideInfo,
    text: &'static str,
    /// A prompt file starts with a note for people, above a `---` line; only what follows it is served.
    prompt: bool,
}

const GUIDES: &[Guide] = &[
    Guide {
        info: GuideInfo {
            name: "agents",
            title: "Working in a PDT project",
            summary: "For agents: finding PDT, tickets, worktrees, runs and tests, and project settings.",
        },
        text: include_str!("../../../docs/agent-guide.md"),
        prompt: false,
    },
    Guide {
        info: GuideInfo {
            name: "run-config",
            title: "Run configuration",
            summary: "How runs, tests and areas are configured: .pdt/project.toml and host overrides.",
        },
        text: include_str!("../../../docs/run-config.md"),
        prompt: false,
    },
    Guide {
        info: GuideInfo {
            name: "set-up-project",
            title: "Set up a project's runs",
            summary: "A prompt for an agent session started from PDT: it configures the project's runs and tests.",
        },
        text: include_str!("../../../docs/prompts/set-up-project.md"),
        prompt: true,
    },
    Guide {
        info: GuideInfo {
            name: "set-up-workflow",
            title: "Write a project's agent instructions",
            summary: "A prompt for an agent session started from PDT: it writes the instructions PDT gives agents here.",
        },
        text: include_str!("../../../docs/prompts/set-up-workflow.md"),
        prompt: true,
    },
];

/// Every guide's name, in the list's order.
pub fn guide_names() -> impl Iterator<Item = &'static str> {
    GUIDES.iter().map(|g| g.info.name)
}

/// A guide's text as served, or None for an unknown name.
pub fn guide_text(name: &str) -> Option<&'static str> {
    let g = GUIDES.iter().find(|g| g.info.name == name)?;
    if !g.prompt {
        return Some(g.text);
    }
    let body = g.text.split_once("\n---\n").map_or(g.text, |(_, rest)| rest);
    Some(body.trim_start())
}

pub(crate) async fn list() -> Json<Vec<GuideInfo>> {
    Json(GUIDES.iter().map(|g| g.info.clone()).collect())
}

pub(crate) async fn get_one(Path(name): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let text = guide_text(&name).ok_or_else(|| ApiError::not_found("guide"))?;
    Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_has_text_and_a_unique_name() {
        for (i, g) in GUIDES.iter().enumerate() {
            assert!(guide_text(g.info.name).is_some_and(|t| t.len() > 200), "{}", g.info.name);
            assert!(GUIDES[..i].iter().all(|o| o.info.name != g.info.name), "{} twice", g.info.name);
        }
        assert_eq!(guide_text("nope"), None);
    }

    #[test]
    fn a_prompt_is_served_without_its_note_for_people() {
        let t = guide_text("set-up-project").unwrap();
        assert!(t.starts_with("Set up this project"), "{}", &t[..80]);
        assert!(!t.contains("paste"), "the note for people is left out");
        assert!(guide_text("set-up-workflow").unwrap().starts_with("Help me write"));
    }

    #[test]
    fn guides_name_each_other_by_api_path_not_repo_path() {
        for g in GUIDES {
            let t = guide_text(g.info.name).unwrap();
            assert!(!t.contains("](../") && !t.contains("](prompts/"), "{} links into the repo", g.info.name);
        }
    }
}
