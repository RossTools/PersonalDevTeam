//! An agent that isn't logged in (PDT-604): its login expired, so every turn fails at once. PDT reads the agents'
//! errors (a failed turn's, an exit's; never what an agent writes) against one table of phrases, and confirms a weak
//! match with the agent's login check before it acts: the steps wait instead of being nudged, one Inbox item per agent
//! says how to log in, and the steps go on once the agent is logged in again.

use crate::session::AgentKind;

/// Whose errors a phrase is found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Claude,
    Codex,
    /// The ACP agents, Devin's included (it runs as `devin acp`).
    Acp,
}

impl Family {
    fn of(agent: AgentKind) -> Self {
        match agent {
            AgentKind::Claude => Family::Claude,
            AgentKind::Codex => Family::Codex,
            AgentKind::Devin | AgentKind::Acp(_) => Family::Acp,
        }
    }
}

/// How sure a phrase is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strength {
    /// The API's or the protocol's own words for a refused login (a 401, `authentication_error`, an expired token):
    /// acted on even when the login check says logged in (a login whose token expired can still look logged in).
    Sure,
    /// Words a person could write too ("not logged in"): acted on only when the login check agrees.
    Weak,
}

/// The phrases, lowercase, matched anywhere in an agent's error.
const PATTERNS: &[(Family, Strength, &str)] = &[
    // Claude Code: `Failed to authenticate. API Error: 401 {"type":"error","error":{"type":"authentication_error",…}}`,
    // "OAuth token has expired", "OAuth session expired and could not be refreshed", "Invalid API key · Please run
    // /login".
    (Family::Claude, Strength::Sure, "authentication_error"),
    (Family::Claude, Strength::Sure, "api error: 401"),
    (Family::Claude, Strength::Sure, "oauth token has expired"),
    (Family::Claude, Strength::Sure, "oauth session expired"),
    (Family::Claude, Strength::Sure, "oauth token revoked"),
    (Family::Claude, Strength::Weak, "failed to authenticate"),
    (Family::Claude, Strength::Weak, "invalid api key"),
    (Family::Claude, Strength::Weak, "please run /login"),
    (Family::Claude, Strength::Weak, "not logged in"),
    // Codex: `unexpected status 401 Unauthorized: …`, "Your access token could not be refreshed because your refresh
    // token has expired. Please log out and sign in again.", `token_expired`.
    (Family::Codex, Strength::Sure, "401 unauthorized"),
    (Family::Codex, Strength::Sure, "refresh token has expired"),
    (Family::Codex, Strength::Sure, "refresh token was already used"),
    (Family::Codex, Strength::Sure, "token_expired"),
    (Family::Codex, Strength::Weak, "please log out and sign in again"),
    (Family::Codex, Strength::Weak, "not logged in"),
    (Family::Codex, Strength::Weak, "codex login"),
    // ACP: the protocol's `auth_required` error (-32000 "Authentication required").
    (Family::Acp, Strength::Sure, "authentication required"),
    (Family::Acp, Strength::Sure, "auth_required"),
    (Family::Acp, Strength::Weak, "not logged in"),
];

/// Whether an agent's error says it isn't logged in, and how sure that is: the surest phrase it has.
pub fn login_failure(agent: AgentKind, error: &str) -> Option<Strength> {
    let family = Family::of(agent);
    let text = error.to_lowercase();
    let found: Vec<Strength> =
        PATTERNS.iter().filter(|(f, _, p)| *f == family && text.contains(p)).map(|(_, s, _)| *s).collect();
    if found.contains(&Strength::Sure) { Some(Strength::Sure) } else { found.first().copied() }
}

/// Whether a match is one to act on, given the agent's login check (`logged_in`: None when it can't tell). An agent
/// PDT can't check (an ACP agent) is taken at its word.
pub fn confirmed(strength: Strength, logged_in: Option<bool>) -> bool {
    match (strength, logged_in) {
        (_, Some(false)) | (Strength::Sure, _) => true,
        (Strength::Weak, Some(true)) => false,
        (Strength::Weak, None) => false,
    }
}

/// How to log the agent in again (`label`: its name, an ACP agent's from its definition).
pub fn login_hint(agent: AgentKind, label: &str) -> String {
    match agent {
        AgentKind::Claude => "Run `claude` in a terminal and log in with `/login`.".into(),
        AgentKind::Codex => "Run `codex login` in a terminal.".into(),
        AgentKind::Devin => "Run `devin auth login` in a terminal.".into(),
        AgentKind::Acp(_) => format!("Sign in to {label} in Settings → Agents, or with its own command in a terminal."),
    }
}

/// The Inbox item's title: "Claude isn't logged in: 3 steps are waiting".
pub fn title(label: &str, waiting: usize) -> String {
    match waiting {
        0 => format!("{label} isn't logged in"),
        1 => format!("{label} isn't logged in: 1 step is waiting"),
        n => format!("{label} isn't logged in: {n} steps are waiting"),
    }
}

/// What a conversation shows under the error.
pub fn inline_notice(label: &str, hint: &str) -> String {
    format!("{label} isn't logged in. {hint} Then send your message again; workflow steps go on by themselves.")
}

/// What a step's session is told once its agent is logged in again: its task again, and why.
pub fn retry_message(label: &str, task: &str) -> String {
    format!("{task}\n\nYour last turn failed because {label} wasn't logged in. It is logged in again: carry on.")
}

/// Why a step that waited for the login restarts in a fresh session.
pub fn restart_why(label: &str) -> String {
    format!("Its turn failed because {label} wasn't logged in, and its session couldn't take a message.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AcpAgentId;

    fn acp() -> AgentKind {
        AgentKind::Acp(AcpAgentId::new("antigravity").unwrap())
    }

    /// Errors the agents gave when their login had expired or was missing.
    #[test]
    fn real_login_failures_are_recognised() {
        let claude = [
            r#"Failed to authenticate. API Error: 401 {"type":"error","error":{"type":"authentication_error","message":"OAuth token has expired. Please obtain a new token or refresh your existing token."},"request_id":"req_011CUxQ"}"#,
            "Failed to authenticate: OAuth session expired and could not be refreshed",
            "OAuth token revoked · Please run /login",
        ];
        for e in claude {
            assert_eq!(login_failure(AgentKind::Claude, e), Some(Strength::Sure), "{e}");
        }
        assert_eq!(login_failure(AgentKind::Claude, "Invalid API key · Please run /login"), Some(Strength::Weak));
        let codex = [
            r#"unexpected status 401 Unauthorized: {"error":{"message":"Provided authentication token is expired. Please try signing in again.","code":"token_expired"}}"#,
            "Your access token could not be refreshed because your refresh token has expired. Please log out and sign in again.",
        ];
        for e in codex {
            assert_eq!(login_failure(AgentKind::Codex, e), Some(Strength::Sure), "{e}");
        }
        assert_eq!(login_failure(AgentKind::Codex, "Not logged in. Run `codex login`."), Some(Strength::Weak));
        let a = "Authentication required: No authentication method selected.";
        assert_eq!(login_failure(acp(), a), Some(Strength::Sure));
        assert_eq!(login_failure(AgentKind::Devin, "Authentication required"), Some(Strength::Sure));
        assert_eq!(login_failure(AgentKind::Devin, "Not logged in."), Some(Strength::Weak));
    }

    /// Errors that mention logging in or authenticating without the agent being logged out, and other agents' words.
    #[test]
    fn other_errors_are_not() {
        let normal = [
            "API Error: 529 Overloaded",
            "usage limit reached",
            "The tests authenticate against a fake server; 3 of them failed.",
            "Claude stopped unexpectedly (exit code 1).",
            "git push failed: could not read Username for 'https://github.com'",
        ];
        for e in normal {
            for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Devin, acp()] {
                assert_eq!(login_failure(agent, e), None, "{e} for {agent:?}");
            }
        }
        assert_eq!(login_failure(AgentKind::Claude, "401 Unauthorized"), None, "Codex's words, not Claude's");
        assert_eq!(login_failure(AgentKind::Codex, "authentication_error"), None);
    }

    #[test]
    fn a_weak_match_needs_the_login_check() {
        assert!(confirmed(Strength::Sure, Some(true)), "an expired token can still look logged in");
        assert!(confirmed(Strength::Sure, None));
        assert!(confirmed(Strength::Weak, Some(false)));
        assert!(!confirmed(Strength::Weak, Some(true)));
        assert!(!confirmed(Strength::Weak, None));
    }

    #[test]
    fn the_item_counts_its_steps() {
        assert_eq!(title("Claude", 0), "Claude isn't logged in");
        assert_eq!(title("Claude", 1), "Claude isn't logged in: 1 step is waiting");
        assert_eq!(title("Codex", 3), "Codex isn't logged in: 3 steps are waiting");
        assert!(login_hint(AgentKind::Claude, "Claude").contains("/login"));
        assert!(login_hint(acp(), "Antigravity").contains("Antigravity"));
    }
}
