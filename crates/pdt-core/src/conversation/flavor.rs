//! A defined ACP agent's extensions ([`AcpFlavor`]): plain ACP, or what its built-in profile's agent adds, mapped in
//! that agent's module (`super::omp`, `super::cursor`, `super::grok`, `super::antigravity`).

use super::acp::{AcpExtensions, ExtAnswer, ExtRequest, PlainAcp};
use super::antigravity::{self, AntigravityExtensions};
use super::cursor::{self, CursorExtensions};
use super::grok::{self, GrokExtensions};
use super::omp::{self, OmpExtensions};
use crate::acp_agent::AcpFlavor;
use serde_json::Value;

macro_rules! by_flavor {
    ($self:ident, $method:ident($($arg:expr),*)) => {
        match $self {
            AcpFlavor::Plain => PlainAcp.$method($($arg),*),
            AcpFlavor::Omp => OmpExtensions.$method($($arg),*),
            AcpFlavor::Cursor => CursorExtensions.$method($($arg),*),
            AcpFlavor::Grok => GrokExtensions.$method($($arg),*),
            AcpFlavor::Antigravity => AntigravityExtensions.$method($($arg),*),
        }
    };
}

impl AcpExtensions for AcpFlavor {
    fn name(&self) -> &str {
        by_flavor!(self, name())
    }

    fn tool_name(&self, update: &Value) -> Option<String> {
        by_flavor!(self, tool_name(update))
    }

    fn question_in_title(&self) -> bool {
        by_flavor!(self, question_in_title())
    }

    fn other_field(&self, key: &str) -> Option<String> {
        by_flavor!(self, other_field(key))
    }

    fn pdt_tool_prefix(&self) -> Option<&str> {
        by_flavor!(self, pdt_tool_prefix())
    }

    fn resolves_symlinks(&self) -> bool {
        by_flavor!(self, resolves_symlinks())
    }

    fn rejected(&self, update: &Value) -> bool {
        by_flavor!(self, rejected(update))
    }

    fn session_option(&self) -> Option<&str> {
        by_flavor!(self, session_option())
    }

    fn raw_output(&self, update: &Value) -> Option<(String, Option<i32>)> {
        by_flavor!(self, raw_output(update))
    }

    fn ext_request(&self, method: &str, params: &Value) -> Option<ExtRequest> {
        by_flavor!(self, ext_request(method, params))
    }

    fn ext_answer(&self, method: &str, answer: &ExtAnswer) -> Value {
        by_flavor!(self, ext_answer(method, answer))
    }
}

impl AcpFlavor {
    /// `session/load` failed with `code` and `message` because the agent has no saved session by that id (the
    /// spec's resource-not-found, or the agent's own way of saying it).
    pub fn session_missing(self, code: i64, message: &str) -> bool {
        code == -32002
            || match self {
                AcpFlavor::Plain => false,
                AcpFlavor::Omp => omp::session_missing(code, message),
                AcpFlavor::Cursor => cursor::session_missing(code, message),
                AcpFlavor::Grok => grok::session_missing(code, message),
                AcpFlavor::Antigravity => false,
            }
    }

    /// Why a session could not start, in words the user can act on, when the agent's error (`code`, `message`) means
    /// more than it says: Antigravity's "Authentication required" means it isn't signed in.
    pub fn refusal_hint(self, code: i64, message: &str) -> Option<String> {
        match self {
            AcpFlavor::Antigravity if antigravity::needs_sign_in(code, message) => Some(
                "Antigravity is not signed in: sign in from Settings → Agents → Antigravity (its own Google sign-in, \
                 apart from the agy CLI's)"
                    .into(),
            ),
            _ => None,
        }
    }
}
