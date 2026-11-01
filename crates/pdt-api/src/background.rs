//! The loops and tasks pdtd runs in the background for as long as it runs, started once it serves (after the startup
//! resume and the poll, which pdtd starts itself). A new background loop goes here, not in pdtd.

use crate::AppState;
use crate::loops::{Start, run_every};
use std::time::Duration;

/// Starts them (pdtd, once).
pub fn start_background(state: &AppState) {
    // The forge logins, every 5 minutes (PDT-252).
    tokio::spawn(run_every(
        state.clone(),
        "forge logins",
        crate::forge_login::CHECK_EVERY,
        Start::AfterOnePeriod,
        |st| async move { crate::forge_login::check(&st).await },
    ));
    // Each active attempt's changed files and the overlaps they make, every 30 s (tickets spec §7).
    tokio::spawn(run_every(state.clone(), "overlaps", Duration::from_secs(30), Start::Now, |st| async move {
        crate::refresh_overlaps(&st).await
    }));
    // The workflow engine (Phase 7a): new events start runs, runs go on, messages wait for their agent session to be
    // idle. Built-in workflows (PR notices, finishing merged tickets) are installed on the first tick.
    tokio::spawn(run_every(state.clone(), "engine", Duration::from_secs(2), Start::Now, |st| async move {
        crate::run_workflows(&st).await
    }));
    // Time-based activity signals (no output while working, a Claude hook that never came).
    tokio::spawn(crate::run_activity_ticker(state.clone()));
    tokio::spawn(crate::run_conversation_ticker(state.clone()));
    // Notifications (PDT-108): rows made from sessions' and the Inbox's state, cleared when their cause goes away.
    tokio::spawn(crate::run_notifier(state.clone()));
    // The local forge's CI runs (PDT-178): one that ends has its attempt read again at once.
    tokio::spawn(crate::run_local_ci_watch(state.clone()));
    // macOS asks for the Local Network permission now, while the user is likely at the Mac (PDT-156).
    tokio::spawn({
        let st = state.clone();
        async move { crate::prompt_local_network(&st).await }
    });
    // What the integrations run for as long as pdtd does (PDT-304: the Discord bot's Gateway connection).
    crate::start_services(state);
    // Auto-start of To do / Ready tickets, per project policy (Phase 7c).
    tokio::spawn(crate::run_pickup_watch(state.clone()));
    // Worktrees' sizes, and the cleanup of finished tickets' build output (PDT-324).
    tokio::spawn(crate::run_storage(state.clone()));
    // Warn at startup when an agent CLI is missing or not logged in for the user this daemon runs as (spec §3.1).
    {
        let st = state.clone();
        const STARTUP_CHECK_RETRIES: usize = 3;
        tokio::spawn(async move {
            let mut c = crate::run_host_checks(&st).await;
            // An agent's check that did not finish (a slow first launch while sessions restart) runs again, so the
            // web does not go on with a stale answer until someone presses "Run checks again".
            for _ in 0..STARTUP_CHECK_RETRIES {
                if !c.agents_inconclusive() || st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                c = crate::run_host_checks(&st).await;
            }
            for (name, check) in [("claude", &c.claude), ("codex", &c.codex), ("devin", &c.devin)] {
                // Devin is optional: only an installed one that is not logged in is worth a warning.
                if !check.ok && (check.found || name != "devin") {
                    tracing::warn!(tool = name, detail = %check.detail, "agent CLI is missing or not logged in for this user");
                }
            }
        });
    }
    // Hourly update checks (and the auto-when-idle install); returns at once unless pdt-host supervises this pdtd.
    // A panic in it is logged and it starts again a minute later.
    {
        let st = state.clone();
        tokio::spawn(async move {
            while crate::loops::contain("the update loop", None, crate::run_update_loop(st.clone())).await
                == crate::loops::Ended::Panicked
                && !st.shutting_down.load(std::sync::atomic::Ordering::SeqCst)
            {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }
}
