//! The PM's watches.

use super::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Unwatch {
    /// The watch's id.
    id: String,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("watch", "Watch a ticket, workflow run or document").pm(),
            title: "Watch a ticket, run or document",
            description: "Tells this conversation when something happens to a ticket, a workflow run or a document: \
                          PDT sends a `[PDT] Watched …` message once you are idle (several within a minute in one; a \
                          document's edits once they stop for a few minutes). `events` for a ticket or run: \
                          `finished` (Done, Won't do, merged; a run succeeded, failed or was cancelled), `outcome` \
                          (an agent step reported), `comment` (someone else commented), `needs_you` (it waits on \
                          the user), `failed` (CI failed, the merge queue dropped it, the run failed); default \
                          finished, failed and needs_you. For a `document` (its id or exact title; one outside PDT \
                          must be imported or linked first): `changed` (its content changed; the message says what \
                          changed and who edited it, where the provider says) and `comment` (a new comment or \
                          reply), both by default. With `once` (default true; false for a document, which never \
                          finishes) the watch ends when the target finishes (one already finished is refused); a \
                          document's ends when it is deleted or removed from PDT. Use it when the user asks you to \
                          keep an eye on something, instead of checking again and again. It asks nobody: only this \
                          conversation is told.",
            schema: schema::<crate::watches::NewWatch>,
        },
        Tool {
            decl: ToolDecl::new("unwatch", "Stop watching a ticket, run or document").pm(),
            title: "Stop watching",
            description: "Ends one of this conversation's watches, by its id (list_watches gives them).",
            schema: schema::<Unwatch>,
        },
        Tool {
            decl: ToolDecl::new("list_watches", "List this conversation's watches").read_only().pm(),
            title: "List watches",
            description: "This conversation's watches: what each watches, what it hears about, and whether it ends \
                          once the target finishes.",
            schema: schema::<NoArgs>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "watch" => {
            let a: crate::watches::NewWatch = args(a)?;
            let w = crate::watches::watch(st, &a, Some(&cx.session.id)).await.map_err(api)?;
            let info = crate::watches::infos(st, vec![w]).await.map_err(api)?;
            Ok(json!({"watching": info}))
        }
        "unwatch" => {
            let a: Unwatch = args(a)?;
            crate::watches::unwatch(st, &a.id, Some(&cx.session.id)).await.map_err(api)?;
            Ok(json!({"unwatched": a.id}))
        }
        "list_watches" => {
            let _: NoArgs = args(a)?;
            let watches = st.store.watches_by(Some(&cx.session.id)).await.map_err(store)?;
            Ok(json!({"watches": crate::watches::infos(st, watches).await.map_err(api)?}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}
