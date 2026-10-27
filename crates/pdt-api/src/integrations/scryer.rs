//! Scryer (films, series and anime): its connection, the Scryer actions and the PM's tools.

use super::{Integration, Tool};
use crate::mcp::tools::{Cx, confirmable};
use crate::workflow::catalog::{BoxFut, NodeKind};
use pdt_core::integration::{ConnectionKind, ToolDecl};
use pdt_core::pm::AskTarget;
use pdt_core::scryer::SCRYER;
use serde_json::Value;

pub struct Scryer;

impl Integration for Scryer {
    fn id(&self) -> &'static str {
        "scryer"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: SCRYER.into(),
            label: "Scryer".into(),
            plural: "Scryer".into(),
            setup: "A Scryer media manager (films, series and anime): the PM browses its library, looks titles up and \
                    adds them, and helps set Scryer up (indexers, download clients, settings) through its API; Search, \
                    Add, queue, library, Query and Change steps use it."
                .into(),
            description: "Films, series and anime: library, search, add, what’s downloading (the PM and the Scryer \
                          actions)."
                .into(),
            url: true,
            url_placeholder: Some("http://192.168.1.10:8080".into()),
            url_hint: Some("Scryer’s address, with its base path if it has one (http://nas/scryer).".into()),
            secret_label: "API key".into(),
            secret_hint: "In Scryer: Settings → API keys → Create. It starts with ska_.".into(),
            pm_acts: Some(
                "The PM adds titles and changes settings without asking (deleting, resetting and passing on a \
                 credential still ask)"
                    .into(),
            ),
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        crate::scryer::kinds()
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                decl: ToolDecl::new("scryer_library", "Browse the Scryer media library").read_only().open_world().pm(),
                title: "Browse the Scryer library",
                description: "Titles in the user's Scryer media library: films, series and anime, by facet, words in the \
                              title and whether they are monitored, a page at a time (limit, offset), with each one's \
                              library, status and episodes on disk.",
                schema: crate::scryer::tool_schema::<crate::scryer::ScryerLibraryConfig>,
            },
            Tool {
                decl: ToolDecl::new("scryer_search", "Look up films, series and anime to add to Scryer")
                    .read_only()
                    .open_world()
                    .pm(),
                title: "Look up titles to add to Scryer",
                description: "Searches Scryer's metadata for films, series and anime by title (one facet, or all): each \
                              result's ids (tvdbId, tmdbId, imdbId) for scryer_add, and whether it is in the library \
                              already.",
                schema: crate::scryer::tool_schema::<crate::scryer::ScryerSearchConfig>,
            },
            Tool {
                decl: ToolDecl::new("scryer_queue", "See what Scryer is downloading").read_only().open_world().pm(),
                title: "See Scryer's downloads",
                description: "What Scryer is downloading: each item's title, progress, status, time left and client, \
                              and why one needs the user.",
                schema: crate::scryer::tool_schema::<crate::scryer::ScryerQueueConfig>,
            },
            Tool {
                decl: ToolDecl::new("scryer_add", "Add a film, series or anime to Scryer").asks().open_world().pm(),
                title: "Add to Scryer",
                description: "Adds a film, series or anime to the user's Scryer library by the ids scryer_search gave \
                              (facet and at least one id; give name too so the user sees it), monitored unless told, so \
                              Scryer finds and downloads it. A title already in the library is left as it is. The user \
                              confirms each call unless the Scryer connection lets you act.",
                schema: crate::scryer::tool_schema::<crate::scryer::ScryerAddConfig>,
            },
            Tool {
                decl: ToolDecl::new("scryer_schema", "Find Scryer's settings and operations in its API")
                    .read_only()
                    .open_world()
                    .pm(),
                title: "Look up Scryer's API",
                description: "Finds and describes Scryer's GraphQL operations and types, to read or change any of its \
                              settings (indexers, download clients, seeding profiles, quality, general settings, jobs): \
                              `search` words in names and descriptions; `operation` one query or mutation with its \
                              arguments, the input types they take and what it returns; `type` one type's fields. With \
                              none, how big the API is.",
                schema: crate::scryer::tool_schema::<crate::scryer::graphql::ScryerSchemaArgs>,
            },
            Tool {
                decl: ToolDecl::new(
                    "scryer_query",
                    "Read anything from Scryer's API (settings, indexers, download clients)",
                )
                .read_only()
                .open_world()
                .pm(),
                title: "Read from Scryer",
                description: "Runs a GraphQL query on Scryer (a query document and its variables), checked against \
                              Scryer's schema first; mutations go through scryer_mutate. Credentials in the answer are \
                              redacted, and a long answer is cut (it says where): select fewer fields or page.",
                schema: crate::scryer::tool_schema::<crate::scryer::graphql::ScryerQueryConfig>,
            },
            Tool {
                decl: ToolDecl::new("scryer_mutate", "Change Scryer's settings through its API")
                    .asks()
                    .open_world()
                    .pm(),
                title: "Change Scryer",
                description: "Runs one GraphQL mutation on Scryer: its name, its arguments by name and the fields to read \
                              back (its scalar fields when left out), checked against Scryer's schema first. Put a \
                              password or API key in as {\"$secret\": \"<connection>\"}: PDT fills in that connection's \
                              secret, so it never passes through you. The user confirms each call unless the Scryer \
                              connection lets you act; deleting, resetting, upgrades and calls that send a secret always \
                              ask.",
                schema: crate::scryer::tool_schema::<crate::scryer::graphql::ScryerMutateConfig>,
            },
        ]
    }

    fn call_tool<'a>(&'a self, cx: &'a Cx<'a>, name: &'a str, a: Value) -> BoxFut<'a, Result<Value, String>> {
        let st = cx.st;
        Box::pin(async move {
            match name {
                "scryer_library" => crate::scryer::library_tool(st, a).await,
                "scryer_search" => crate::scryer::search_tool(st, a).await,
                "scryer_queue" => crate::scryer::queue_tool(st, a).await,
                "scryer_add" => {
                    let given = a.get("connection").and_then(Value::as_str).map(String::from);
                    confirmable(cx, name, &AskTarget::Connection { kind: SCRYER, name: given }).await?;
                    crate::scryer::add_tool(st, a).await
                }
                "scryer_schema" => crate::scryer::graphql::schema_tool(st, a).await,
                "scryer_query" => crate::scryer::graphql::query_tool(st, a).await,
                "scryer_mutate" => {
                    let target = pdt_core::pm::ask_target(name, &a).unwrap_or(AskTarget::Always);
                    confirmable(cx, name, &target).await?;
                    crate::scryer::graphql::mutate_tool(st, a).await
                }
                other => Err(format!("no tool is called {other}")),
            }
        })
    }
}
