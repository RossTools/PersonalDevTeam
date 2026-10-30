//! The Get started checklist (PDT-230): its four steps, each ticked off from the host's real state, and whether it
//! opens on launch. Pure: pdt-api gathers the facts and stores the state.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the host remembers about the checklist. Stored once, the first time anyone asks for it: an install that
/// already has a project and an agent then never sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SetupState {
    /// This host needs setting up: false for an install that was already in use when the checklist arrived.
    pub needed: bool,
    /// Dismissed by the user: it no longer opens on launch (Settings › This host › Setup reopens it).
    pub dismissed: bool,
    /// Every step was done once: it no longer opens on launch, even if a step comes undone later.
    pub finished: bool,
    /// "Skip, my projects aren't on GitHub": the GitHub step counts as done.
    pub github_skipped: bool,
    /// The default agent was picked (in the checklist or in Settings), or the preset one confirmed.
    pub default_agent_confirmed: bool,
}

impl SetupState {
    /// The state a host starts with: needed unless it already has a project and an agent.
    pub fn first_seen(f: &SetupFacts) -> Self {
        Self { needed: !(f.projects > 0 && f.any_agent()), ..Self::default() }
    }

    /// Whether the app opens on the checklist.
    pub fn opens_on_launch(&self) -> bool {
        self.needed && !self.dismissed && !self.finished
    }

    pub fn apply(&self, p: &SetupPatch) -> Self {
        let mut s = self.clone();
        if let Some(v) = p.dismissed {
            s.dismissed = v;
        }
        if let Some(v) = p.github_skipped {
            s.github_skipped = v;
        }
        if let Some(v) = p.default_agent_confirmed {
            s.default_agent_confirmed = v;
        }
        s
    }
}

/// A change to the checklist's state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupPatch {
    #[serde(default)]
    #[ts(optional)]
    pub dismissed: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub github_skipped: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub default_agent_confirmed: Option<bool>,
}

/// The host's real state, as the steps read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupFacts {
    pub git: bool,
    pub claude: bool,
    pub codex: bool,
    pub devin: bool,
    pub github_connected: bool,
    /// The GitHub plugin is turned off (PDT-251): nothing to connect.
    pub github_off: bool,
    pub projects: usize,
}

impl SetupFacts {
    pub fn any_agent(&self) -> bool {
        self.claude || self.codex || self.devin
    }
}

/// Where a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StepState {
    Todo,
    Done,
    /// Done by skipping it (GitHub only).
    Skipped,
}

impl StepState {
    pub fn complete(self) -> bool {
        self != StepState::Todo
    }

    fn done_if(done: bool) -> Self {
        if done { StepState::Done } else { StepState::Todo }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupSteps {
    /// git and at least one agent are found.
    pub agents: StepState,
    /// Connected, or skipped (also while the GitHub plugin is off).
    pub github: StepState,
    /// One project exists.
    pub project: StepState,
    /// The default agent was set explicitly or confirmed.
    pub default_agent: StepState,
}

impl SetupSteps {
    pub fn read(s: &SetupState, f: &SetupFacts) -> Self {
        Self {
            agents: StepState::done_if(f.git && f.any_agent()),
            // Connected wins over skipped: it says more.
            github: if f.github_connected {
                StepState::Done
            } else if s.github_skipped || f.github_off {
                StepState::Skipped
            } else {
                StepState::Todo
            },
            project: StepState::done_if(f.projects > 0),
            default_agent: StepState::done_if(s.default_agent_confirmed),
        }
    }

    /// How many of the four are done.
    pub fn done(&self) -> u32 {
        [self.agents, self.github, self.project, self.default_agent].iter().filter(|s| s.complete()).count() as u32
    }

    pub fn all_done(&self) -> bool {
        self.done() == 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> SetupFacts {
        SetupFacts::default()
    }

    #[test]
    fn a_new_install_needs_setting_up() {
        let s = SetupState::first_seen(&facts());
        assert!(s.needed && s.opens_on_launch());
        // An agent alone, or a project without one, is still a new install.
        assert!(SetupState::first_seen(&SetupFacts { claude: true, git: true, ..facts() }).needed);
        assert!(SetupState::first_seen(&SetupFacts { projects: 1, ..facts() }).needed);
    }

    #[test]
    fn an_install_with_a_project_and_an_agent_never_sees_it() {
        let s = SetupState::first_seen(&SetupFacts { codex: true, projects: 2, ..facts() });
        assert!(!s.needed);
        assert!(!s.opens_on_launch());
    }

    #[test]
    fn dismissed_or_finished_no_longer_opens_on_launch() {
        let s = SetupState::first_seen(&facts());
        let dismissed = s.apply(&SetupPatch { dismissed: Some(true), ..Default::default() });
        assert!(!dismissed.opens_on_launch());
        // Reopened from Settings.
        assert!(dismissed.apply(&SetupPatch { dismissed: Some(false), ..Default::default() }).opens_on_launch());
        assert!(!SetupState { finished: true, ..s }.opens_on_launch());
    }

    #[test]
    fn the_agents_step_needs_git_and_one_agent() {
        let s = SetupState::default();
        let step = |f: SetupFacts| SetupSteps::read(&s, &f).agents;
        assert_eq!(step(facts()), StepState::Todo);
        assert_eq!(step(SetupFacts { claude: true, ..facts() }), StepState::Todo, "no git");
        assert_eq!(step(SetupFacts { git: true, ..facts() }), StepState::Todo, "no agent");
        for f in [
            SetupFacts { git: true, claude: true, ..facts() },
            SetupFacts { git: true, codex: true, ..facts() },
            SetupFacts { git: true, devin: true, ..facts() },
        ] {
            assert_eq!(step(f), StepState::Done);
        }
    }

    #[test]
    fn github_is_done_when_connected_or_skipped() {
        let skipped = SetupState { github_skipped: true, ..Default::default() };
        let gh = |s: &SetupState, connected| {
            SetupSteps::read(s, &SetupFacts { github_connected: connected, ..facts() }).github
        };
        assert_eq!(gh(&SetupState::default(), false), StepState::Todo);
        assert_eq!(gh(&skipped, false), StepState::Skipped);
        assert_eq!(gh(&SetupState::default(), true), StepState::Done);
        assert_eq!(gh(&skipped, true), StepState::Done);
        assert!(StepState::Skipped.complete());
        let off = SetupFacts { github_off: true, ..facts() };
        assert_eq!(SetupSteps::read(&SetupState::default(), &off).github, StepState::Skipped, "GitHub turned off");
    }

    #[test]
    fn project_and_default_agent_steps_read_real_state() {
        let s = SetupState::default();
        let steps = SetupSteps::read(&s, &SetupFacts { projects: 1, ..facts() });
        assert_eq!((steps.project, steps.default_agent), (StepState::Done, StepState::Todo));
        let confirmed = SetupState { default_agent_confirmed: true, ..s };
        assert_eq!(SetupSteps::read(&confirmed, &facts()).default_agent, StepState::Done);
    }

    #[test]
    fn counts_the_steps_done() {
        let none = SetupSteps::read(&SetupState::default(), &facts());
        assert_eq!((none.done(), none.all_done()), (0, false));
        let s = SetupState { github_skipped: true, default_agent_confirmed: true, ..Default::default() };
        let all = SetupSteps::read(&s, &SetupFacts { git: true, codex: true, projects: 1, ..facts() });
        assert_eq!((all.done(), all.all_done()), (4, true));
    }

    #[test]
    fn stored_state_reads_back_with_defaults() {
        let s: SetupState = serde_json::from_str(r#"{"needed":true}"#).unwrap();
        assert_eq!(s, SetupState { needed: true, ..Default::default() });
    }
}
