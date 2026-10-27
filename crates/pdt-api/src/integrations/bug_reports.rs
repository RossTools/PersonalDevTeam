//! Bug reports about PDT itself (`report_bug`): an issue on PDT's repository, or a Draft ticket on the instance PDT
//! is developed on, as Host settings route it.

use super::{Integration, Tool};
use crate::mcp::tools::{Cx, api, args, confirmable, schema};
use crate::workflow::catalog::BoxFut;
use pdt_core::integration::ToolDecl;
use pdt_core::pm::AskTarget;
use serde_json::Value;

pub struct BugReports;

impl Integration for BugReports {
    fn id(&self) -> &'static str {
        "bug_reports"
    }

    fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            decl: ToolDecl::new("report_bug", "Report a bug in PDT (an issue on its repository, or a local ticket)")
                .asks()
                .open_world()
                .pm(),
            title: "Report a bug in PDT",
            description: "Reports a bug in PDT itself (not in the project you work on), as Host settings route it: an \
                              issue on PDT's GitHub repository, or a Draft bug ticket on the instance PDT is developed on. \
                              includeDiagnostics adds this instance's version, OS, update state and the end of its log, \
                              redacted. The user confirms each call.",
            schema: schema::<crate::bug_reports::NewBugReport>,
        }]
    }

    fn call_tool<'a>(&'a self, cx: &'a Cx<'a>, name: &'a str, a: Value) -> BoxFut<'a, Result<Value, String>> {
        let st = cx.st;
        Box::pin(async move {
            match name {
                "report_bug" => {
                    let mut a: crate::bug_reports::NewBugReport = args(a)?;
                    confirmable(cx, name, &AskTarget::Always).await?;
                    // A retried call within a minute files nothing new.
                    a.nonce = Some(format!("mcp:{}:{}", cx.session.id, a.title.trim()));
                    let filed = crate::bug_reports::file(st, a, Some(cx.session)).await.map_err(api)?;
                    Ok(serde_json::to_value(filed).map_err(|e| e.to_string())?)
                }
                other => Err(format!("no tool is called {other}")),
            }
        })
    }
}
