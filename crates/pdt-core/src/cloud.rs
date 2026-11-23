//! Cloud instances (PDT-638): a remote instance that is a VM at a cloud provider (AWS, Azure), which the owner starts
//! when a ticket is placed on it and stops once it has been idle a while, metering what it costs. The provider is
//! reached through a connection of its kind (its credentials); `machine` names the VM there (an EC2 instance id, an
//! Azure `resource-group/vm-name`). This module holds the rules: what one round of the owner's check does
//! ([`step`]) and the cost meter. The providers and the loop are pdt-api's (`cloud`).

use crate::link::InstanceFacts;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Idle minutes before a cloud instance is stopped, when the user sets none.
pub const DEFAULT_IDLE_MINUTES: u32 = 30;
/// The most idle minutes the user may set (a day); 0 turns auto-stop off.
pub const MAX_IDLE_MINUTES: u32 = 24 * 60;
/// How long a started instance has to come online (its PDT answering heartbeats) before the owner says it didn't.
/// Windows takes up to about 3 minutes from deallocated; this leaves room for updates on boot.
pub const START_TIMEOUT_MS: i64 = 10 * 60 * 1000;
/// How long the owner leaves a start or stop it asked for to take effect before it asks again.
pub const RETRY_AFTER_MS: i64 = 3 * 60 * 1000;
/// The longest gap between two checks the meter counts as running: a longer one (pdtd wasn't running) counts as this.
pub const METER_GAP_CAP_MS: i64 = 5 * 60 * 1000;

/// A VM's power state, as its provider reports it (simplified).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Power {
    Running,
    Starting,
    Stopping,
    /// Stopped and not billed for compute (Azure: deallocated).
    Stopped,
    /// Shut down from inside its OS but still allocated, so still billed for compute (Azure's `stopped`). The owner
    /// deallocates it unless work waits for it, which starts it instead.
    Halted,
    /// Not read yet, or the provider said something PDT doesn't know.
    Unknown,
}

impl Power {
    pub fn as_str(self) -> &'static str {
        match self {
            Power::Running => "running",
            Power::Starting => "starting",
            Power::Stopping => "stopping",
            Power::Stopped => "stopped",
            Power::Halted => "halted",
            Power::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Power {
        match s {
            "running" => Power::Running,
            "starting" => Power::Starting,
            "stopping" => Power::Stopping,
            "stopped" => Power::Stopped,
            "halted" => Power::Halted,
            _ => Power::Unknown,
        }
    }

    /// Whether the provider bills compute for it in this state.
    pub fn billed(self) -> bool {
        matches!(self, Power::Running | Power::Starting | Power::Stopping | Power::Halted)
    }
}

/// What the user sets for a cloud instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloudSetup {
    /// The connection (by name) whose kind is the provider and whose secret is its credentials.
    pub connection: String,
    /// The VM at the provider: an EC2 instance id (`i-0abc…`), an Azure `resource-group/vm-name`.
    pub machine: String,
    /// Stop it after this many minutes without sessions, runs or work waiting for it; 0: never.
    #[serde(default = "default_idle")]
    pub idle_minutes: u32,
    /// What it costs an hour while it runs, in US dollars (the meter's rate).
    #[serde(default)]
    pub hourly_rate: f64,
}

fn default_idle() -> u32 {
    DEFAULT_IDLE_MINUTES
}

impl CloudSetup {
    /// Trims it, or says what is wrong with it.
    pub fn checked(mut self) -> Result<Self, String> {
        self.connection = self.connection.trim().to_string();
        self.machine = self.machine.trim().to_string();
        if self.connection.is_empty() {
            return Err("Pick the connection with the provider's credentials.".into());
        }
        if self.machine.is_empty() {
            return Err("Name the machine at the provider.".into());
        }
        if self.idle_minutes > MAX_IDLE_MINUTES {
            return Err(format!("Stop it after at most {MAX_IDLE_MINUTES} idle minutes (0: never)."));
        }
        if !self.hourly_rate.is_finite() || self.hourly_rate < 0.0 || self.hourly_rate > 1000.0 {
            return Err("The hourly rate is dollars an hour, between 0 and 1000.".into());
        }
        Ok(self)
    }
}

/// What the owner keeps between its checks of a cloud instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloudState {
    pub power: Option<Power>,
    /// When the provider was last asked (the meter counts from it), in ms.
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
    /// When it last had sessions, runs or work waiting for it (or came up), in ms: idle counts from it.
    #[ts(type = "number | null")]
    pub last_busy_at: Option<i64>,
    /// When the owner asked the provider to start it, until it is online; in ms.
    #[ts(type = "number | null")]
    pub start_requested_at: Option<i64>,
    /// When the owner asked the provider to stop it, until it is stopped; in ms.
    #[ts(type = "number | null")]
    pub stop_requested_at: Option<i64>,
    /// What went wrong last (the provider refused, it didn't come online), until a check goes well.
    pub last_error: Option<String>,
}

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub power: Power,
    /// Sessions or runs are live on it.
    pub busy: bool,
    /// Pickup holds tickets placed on it.
    pub wanted: bool,
    /// Its PDT answers heartbeats.
    pub connected: bool,
    pub now: i64,
}

/// What the owner asks the provider to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudAction {
    Start,
    /// Stop it, deallocated where the provider tells the two apart.
    Stop,
}

/// One check's outcome: the state to keep, what to ask the provider for, and the running time to meter.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub state: CloudState,
    pub action: Option<CloudAction>,
    pub metered_ms: i64,
}

fn recent(at: Option<i64>, now: i64) -> bool {
    at.is_some_and(|at| now - at < RETRY_AFTER_MS)
}

/// One round of the owner's check: a stopped instance with work waiting starts; a running one with nothing live on
/// it and none waiting stops after `idle_minutes`. A start or stop the owner asked for isn't asked again for
/// [`RETRY_AFTER_MS`]; a start that hasn't brought it online within [`START_TIMEOUT_MS`] is an error (and asked again).
pub fn step(setup: &CloudSetup, prev: &CloudState, o: &Observed) -> Step {
    let now = o.now;
    let metered_ms = match (prev.power, prev.checked_at) {
        (Some(p), Some(at)) if p.billed() && o.power.billed() => (now - at).clamp(0, METER_GAP_CAP_MS),
        _ => 0,
    };
    let came_up = o.power == Power::Running && prev.power != Some(Power::Running);
    let mut s = CloudState {
        power: Some(o.power),
        checked_at: Some(now),
        last_busy_at: if o.busy || o.wanted || came_up || prev.last_busy_at.is_none() {
            Some(now)
        } else {
            prev.last_busy_at
        },
        start_requested_at: prev.start_requested_at.filter(|_| !o.connected),
        stop_requested_at: prev.stop_requested_at.filter(|_| o.power != Power::Stopped),
        last_error: None,
    };
    if let Some(at) = s.start_requested_at
        && now - at >= START_TIMEOUT_MS
    {
        s.last_error =
            Some(format!("it was started {} minutes ago and its PDT hasn't answered yet", (now - at) / 60_000));
    }
    let action = match o.power {
        Power::Stopped | Power::Halted if o.wanted && !recent(s.start_requested_at, now) => Some(CloudAction::Start),
        // Shut down from inside but billed: nothing runs on it, so deallocate it whatever its idle minutes.
        Power::Halted if !o.wanted && !recent(s.stop_requested_at, now) => Some(CloudAction::Stop),
        Power::Running if !o.busy && !o.wanted && setup.idle_minutes > 0 && !recent(s.stop_requested_at, now) => {
            let idle = now - s.last_busy_at.unwrap_or(now);
            (idle >= i64::from(setup.idle_minutes) * 60_000).then_some(CloudAction::Stop)
        }
        _ => None,
    };
    match action {
        Some(CloudAction::Start) => {
            s.start_requested_at = Some(now);
            s.last_busy_at = Some(now);
        }
        Some(CloudAction::Stop) => s.stop_requested_at = Some(now),
        None => {}
    }
    Step { state: s, action, metered_ms }
}

/// Whether the owner is bringing it up: it asked the provider to start it, and it isn't online yet.
pub fn starting(state: &CloudState) -> bool {
    state.start_requested_at.is_some()
}

/// The UTC month of `ms` (`2026-11`): the meter's buckets.
pub fn month_of(ms: i64) -> String {
    let d = jiff::Timestamp::from_millisecond(ms).unwrap_or_default().to_zoned_datetime(jiff::tz::TimeZone::UTC).date();
    format!("{:04}-{:02}", d.year(), d.month())
}

/// What `ms` of running costs at `hourly_rate` dollars an hour.
pub fn cost(ms: i64, hourly_rate: f64) -> f64 {
    ms.max(0) as f64 / 3_600_000.0 * hourly_rate
}

/// A month of a cloud instance's meter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloudUsage {
    /// `2026-11` (UTC).
    pub month: String,
    pub hours: f64,
    /// In US dollars, at the rate set while it ran.
    pub cost: f64,
}

/// The agents installed on an instance that aren't logged in there: it needs a login (over RDP or SSH, or a token
/// from the project's secrets) before they can work.
pub fn needs_login(facts: &InstanceFacts) -> Vec<String> {
    facts.agents.iter().filter(|a| a.installed && !a.logged_in).map(|a| a.agent.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn setup(idle: u32) -> CloudSetup {
        CloudSetup { connection: "aws".into(), machine: "i-1".into(), idle_minutes: idle, hourly_rate: 0.376 }
    }

    fn seen(power: Power, now: i64) -> Observed {
        Observed { power, busy: false, wanted: false, connected: power == Power::Running, now }
    }

    #[test]
    fn a_stopped_instance_with_work_waiting_starts_once() {
        let prev = CloudState { power: Some(Power::Stopped), ..Default::default() };
        let o = Observed { wanted: true, connected: false, ..seen(Power::Stopped, 1_000) };
        let s = step(&setup(30), &prev, &o);
        assert_eq!(s.action, Some(CloudAction::Start));
        assert_eq!(s.state.start_requested_at, Some(1_000));
        assert!(starting(&s.state));
        // The provider still says stopped a moment later: not asked again yet.
        let again = step(&setup(30), &s.state, &Observed { now: 1_000 + MIN, ..o });
        assert_eq!(again.action, None);
        // Long after, it is.
        let later = step(&setup(30), &again.state, &Observed { now: 1_000 + RETRY_AFTER_MS, ..o });
        assert_eq!(later.action, Some(CloudAction::Start));
    }

    #[test]
    fn nothing_waiting_leaves_a_stopped_instance_stopped() {
        let s = step(&setup(30), &CloudState::default(), &seen(Power::Stopped, 5));
        assert_eq!(s.action, None);
        assert_eq!(s.state.power, Some(Power::Stopped));
    }

    #[test]
    fn online_ends_the_start_and_an_idle_instance_stops_after_its_minutes() {
        let prev = CloudState { power: Some(Power::Starting), start_requested_at: Some(0), ..Default::default() };
        let up = step(&setup(30), &prev, &seen(Power::Running, 2 * MIN));
        assert_eq!((up.action, up.state.start_requested_at), (None, None));
        assert_eq!(up.state.last_busy_at, Some(2 * MIN), "idle counts from coming up");
        let busy = step(&setup(30), &up.state, &Observed { busy: true, ..seen(Power::Running, 20 * MIN) });
        assert_eq!((busy.action, busy.state.last_busy_at), (None, Some(20 * MIN)));
        let quiet = step(&setup(30), &busy.state, &seen(Power::Running, 49 * MIN));
        assert_eq!(quiet.action, None, "29 idle minutes");
        let stop = step(&setup(30), &quiet.state, &seen(Power::Running, 50 * MIN));
        assert_eq!(stop.action, Some(CloudAction::Stop));
        assert_eq!(stop.state.stop_requested_at, Some(50 * MIN));
        let again = step(&setup(30), &stop.state, &seen(Power::Stopping, 51 * MIN));
        assert_eq!(again.action, None);
        let down = step(&setup(30), &again.state, &seen(Power::Stopped, 52 * MIN));
        assert_eq!((down.action, down.state.stop_requested_at), (None, None));
    }

    #[test]
    fn work_waiting_keeps_it_running_and_idle_zero_never_stops() {
        let prev = CloudState { power: Some(Power::Running), last_busy_at: Some(0), ..Default::default() };
        let o = Observed { wanted: true, ..seen(Power::Running, 120 * MIN) };
        assert_eq!(step(&setup(30), &prev, &o).action, None);
        assert_eq!(step(&setup(0), &prev, &seen(Power::Running, 900 * MIN)).action, None);
    }

    #[test]
    fn a_start_that_never_comes_online_is_an_error() {
        let prev = CloudState { power: Some(Power::Running), start_requested_at: Some(0), ..Default::default() };
        let o = Observed { connected: false, ..seen(Power::Running, START_TIMEOUT_MS) };
        let s = step(&setup(30), &prev, &o);
        assert!(s.state.last_error.as_deref().unwrap().contains("10 minutes ago"), "{:?}", s.state.last_error);
        assert!(starting(&s.state));
    }

    #[test]
    fn a_vm_shut_down_from_inside_is_metered_and_deallocated_unless_work_waits() {
        let prev = CloudState { power: Some(Power::Running), checked_at: Some(0), ..Default::default() };
        let halted = step(&setup(0), &prev, &seen(Power::Halted, MIN));
        assert_eq!((halted.action, halted.metered_ms), (Some(CloudAction::Stop), MIN), "billed, so deallocated");
        assert_eq!(halted.state.stop_requested_at, Some(MIN));
        let again = step(&setup(0), &halted.state, &seen(Power::Halted, 2 * MIN));
        assert_eq!((again.action, again.metered_ms), (None, MIN), "asked once, still billed");
        let wanted = Observed { wanted: true, ..seen(Power::Halted, 3 * MIN) };
        assert_eq!(step(&setup(30), &CloudState::default(), &wanted).action, Some(CloudAction::Start));
        assert_eq!(Power::parse(Power::Halted.as_str()), Power::Halted);
    }

    #[test]
    fn the_meter_counts_billed_time_between_checks() {
        let prev = CloudState { power: Some(Power::Running), checked_at: Some(0), ..Default::default() };
        assert_eq!(step(&setup(30), &prev, &seen(Power::Running, MIN)).metered_ms, MIN);
        assert_eq!(step(&setup(30), &prev, &seen(Power::Stopped, MIN)).metered_ms, 0, "stopped since");
        assert_eq!(step(&setup(30), &prev, &seen(Power::Running, 60 * MIN)).metered_ms, METER_GAP_CAP_MS);
        let stopped = CloudState { power: Some(Power::Stopped), checked_at: Some(0), ..Default::default() };
        assert_eq!(step(&setup(30), &stopped, &seen(Power::Running, MIN)).metered_ms, 0);
        assert!((cost(90 * MIN, 0.376) - 0.564).abs() < 1e-9);
    }

    #[test]
    fn months_are_utc() {
        assert_eq!(month_of(0), "1970-01");
        // 2026-11-30T23:59:59Z and a second later.
        assert_eq!(month_of(1_796_083_199_000), "2026-11");
        assert_eq!(month_of(1_796_083_200_000), "2026-12");
    }

    #[test]
    fn a_setup_is_checked() {
        let s = CloudSetup { connection: " aws ".into(), machine: " i-1 ".into(), idle_minutes: 30, hourly_rate: 0.3 };
        assert_eq!(s.checked().unwrap().machine, "i-1");
        assert!(CloudSetup { machine: "".into(), ..setup(30) }.checked().is_err());
        assert!(CloudSetup { hourly_rate: -1.0, ..setup(30) }.checked().is_err());
        assert!(setup(MAX_IDLE_MINUTES + 1).checked().is_err());
    }

    #[test]
    fn agents_installed_but_not_logged_in_need_a_login() {
        let fact =
            |agent: &str, installed, logged_in| crate::link::AgentFact { agent: agent.into(), installed, logged_in };
        let facts = InstanceFacts {
            agents: vec![fact("claude", true, false), fact("codex", false, false), fact("devin", true, true)],
            ..Default::default()
        };
        assert_eq!(needs_login(&facts), vec!["claude".to_string()]);
    }
}
