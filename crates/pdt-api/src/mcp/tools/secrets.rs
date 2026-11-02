//! The secrets tools: names only, and commands and files the values go into unseen.

use super::*;

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("list_secrets", "See the names of the secrets it may use").read_only().pm(),
            title: "List the secrets",
            description: "The secrets this session may use: their names and descriptions, never their values.",
            schema: schema::<NoArgs>,
        },
        Tool {
            decl: ToolDecl::new("request_secret", "Ask you for a secret (you type it into the Inbox, never the chat)")
                .pm(),
            title: "Ask the user for a secret",
            description: "Asks the user for a secret (an API key, a token, a password) by name: they type it into PDT's \
                          Inbox, never into this chat. You get `stored: true` once it is (at once if it already is), \
                          never the value: use it with run_with_secrets or write_secret_file. Never ask for a secret \
                          in the chat.",
            schema: schema::<crate::vault::RequestSecret>,
        },
        Tool {
            decl: ToolDecl::new(
                "run_with_secrets",
                "Run a command with secrets in its environment, never pasted into the chat (asks you first)",
            )
            .asks()
            .pm(),
            title: "Run a command with secrets",
            description: "Runs a command line (sh -c; cmd /C on Windows) in this session's folder with secrets in its \
                          environment: an env value `secret:<name>` is that secret's value, and at least one must be. \
                          The user allows each call (unless you run with permissions bypassed). Its output comes back \
                          with every secret's value replaced by `[secret:<name>]`. Each use is recorded.",
            schema: schema::<crate::vault::RunWithSecrets>,
        },
        Tool {
            decl: ToolDecl::new(
                "write_secret_file",
                "Write a file (a .env) with secrets filled in, never pasted into the chat (asks you first)",
            )
            .asks()
            .pm(),
            title: "Write a file with secrets",
            description: "Writes a file (a .env, a config file) in this session's folder (never in .git) from a \
                          template with `{{secret.<name>}}` where each secret goes (at least one), filled in by PDT; \
                          the content is not echoed back. The user allows each call (unless you run with permissions \
                          bypassed). Each use is recorded.",
            schema: schema::<crate::vault::WriteSecretFile>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "list_secrets" => {
            let _: NoArgs = args(a)?;
            crate::vault::list_tool(st, cx.session).await
        }
        "request_secret" => crate::vault::request_tool(st, cx.session, args(a)?).await,
        "run_with_secrets" => {
            let a = args(a)?;
            crate::vault::check_run(cx.session, &a)?;
            asks_first(cx, name).await?;
            crate::vault::run_tool(st, cx.session, a).await
        }
        "write_secret_file" => {
            let a = args(a)?;
            crate::vault::check_write(cx.session, &a)?;
            asks_first(cx, name).await?;
            crate::vault::write_file_tool(st, cx.session, a).await
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// These run in PDT, outside the agent's sandbox and permission mode (PDT-404), so the user allows each call: Claude
/// asks in its own prompt (they are kept out of its allowed tools), other agents through the Inbox. A session that
/// asks before nothing anyway goes ahead, and the PM's "act without asking" never covers them.
async fn asks_first(cx: &Cx<'_>, name: &str) -> Result<(), String> {
    if cx.session.asks_nothing() {
        return Ok(());
    }
    confirmable(cx, name, &pdt_core::pm::AskTarget::Always).await
}
