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
                "Run a command with secrets in its environment (it never sees them)",
            )
            .pm(),
            title: "Run a command with secrets",
            description: "Runs a command line (sh -c; cmd /C on Windows) with secrets in its environment: an env value \
                          `secret:<name>` is that secret's value. Its output comes back with every secret's value \
                          replaced by `[secret:<name>]`. Each use is recorded.",
            schema: schema::<crate::vault::RunWithSecrets>,
        },
        Tool {
            decl: ToolDecl::new("write_secret_file", "Write a file (a .env) with secrets filled in, without seeing it")
                .pm(),
            title: "Write a file with secrets",
            description: "Writes a file (a .env, a config file) from a template with `{{secret.<name>}}` where each \
                          secret goes, filled in by PDT; the content is not echoed back. Each use is recorded.",
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
        "run_with_secrets" => crate::vault::run_tool(st, cx.session, args(a)?).await,
        "write_secret_file" => crate::vault::write_file_tool(st, cx.session, args(a)?).await,
        other => Err(format!("no tool is called {other}")),
    }
}
