import { cn } from '@ui/lib/utils';
import { StatusDot } from '@ui/StatusDot';
import type { Activity } from '../api/gen/Activity';
import type { RunState } from '../api/gen/RunState';
import type { SessionKind } from '../api/gen/SessionKind';
import type { SessionStatus } from '../api/gen/SessionStatus';
import type { ItemInfo } from '../workspaces/useWorkspaces';

/** What an item is: an agent session (`agent_tui`), a shell session, or a run. */
export type AgentStateInput =
  | { kind: SessionKind; status: SessionStatus; activity?: Activity | null; unseen?: boolean }
  | { kind: 'run'; status: RunState; unseen?: boolean };

export type AgentDotKind = 'running' | 'waiting' | 'failed' | 'idle' | 'ended';
export type AgentLabelText = 'Working' | 'Needs you' | 'Done' | 'Idle' | 'Ended' | 'Interrupted' | 'Failed';
/** The text colour of the label: state colours only for state (spec §3). */
export type AgentTone = 'running' | 'waiting' | 'failed' | 'muted';

export interface AgentStateValue {
  dot: AgentDotKind;
  /** Pulses only while an agent (or a run getting ready) is working; never under reduced motion. */
  pulse: boolean;
  label: AgentLabelText | null;
  tone: AgentTone;
  /** Shown as the accent dot next to the state dot. */
  unseen: boolean;
}

const state = (dot: AgentDotKind, label: AgentLabelText | null, tone: AgentTone, unseen: boolean, pulse = false): AgentStateValue =>
  ({ dot, pulse, label, tone, unseen });

/**
 * The one mapping from an item's kind, status, activity and unseen flag to its dot, label and tone (spec §6.5).
 *
 * Sessions: an ended or interrupted one is a hollow dot whatever its activity; a running agent follows its activity
 * (working → green pulsing "Working", needs you → amber "Needs you", idle → grey "Done" while unseen, else "Idle";
 * no or unknown activity → grey, no label); a running shell is green without a pulse or label.
 * Runs: setting up or starting → green pulsing (no label: run views show the run state); ready or running → green;
 * passed → hollow "Done"; failed → red "Failed"; stopped → hollow "Ended".
 */
export function agentState(input: AgentStateInput): AgentStateValue {
  const unseen = input.unseen ?? false;
  if (input.kind === 'run') {
    switch (input.status) {
      case 'setting_up':
      case 'starting': return state('running', null, 'running', unseen, true);
      case 'ready':
      case 'running': return state('running', null, 'running', unseen);
      case 'passed': return state('ended', 'Done', 'muted', unseen);
      case 'failed': return state('failed', 'Failed', 'failed', unseen);
      case 'stopped': return state('ended', 'Ended', 'muted', unseen);
    }
  }
  if (input.status === 'finished') return state('ended', 'Ended', 'muted', unseen);
  if (input.status === 'interrupted') return state('ended', 'Interrupted', 'muted', unseen);
  if (input.kind === 'shell') return state('running', null, 'running', unseen);
  switch (input.activity) {
    case 'working': return state('running', 'Working', 'running', unseen, true);
    case 'needs_you': return state('waiting', 'Needs you', 'waiting', unseen);
    case 'idle': return unseen ? state('idle', 'Done', 'muted', true) : state('idle', 'Idle', 'muted', false);
    default: return state('idle', null, 'muted', unseen);
  }
}

/** The state of a workspace item (a tab or an agent row). */
export function itemState(info: Pick<ItemInfo, 'session' | 'run' | 'unseen'>): AgentStateValue {
  if (info.run) return agentState({ kind: 'run', status: info.run.state, unseen: info.unseen });
  if (info.session) return agentState({ kind: info.session.kind, status: info.session.status, activity: info.session.activity, unseen: info.unseen });
  return state('idle', null, 'muted', info.unseen);
}

const RANK: Partial<Record<AgentLabelText, number>> = { 'Needs you': 5, Failed: 4, Working: 3, Done: 2, Idle: 1 };

/**
 * A group's state (a workspace): its most urgent item — needs you, failed, working, done, idle — with `unseen` when
 * any item is unseen. Items without one of those labels (shells, ended sessions, runs that are up) don't count; null
 * when nothing counts and nothing is unseen.
 */
export function rollup(states: AgentStateValue[]): AgentStateValue | null {
  const unseen = states.some((s) => s.unseen);
  let worst: AgentStateValue | null = null;
  for (const s of states) {
    const r = s.label ? RANK[s.label] : undefined;
    if (r !== undefined && (worst === null || r > RANK[worst.label!]!)) worst = s;
  }
  if (worst) return { ...worst, unseen };
  return unseen ? state('idle', null, 'muted', true) : null;
}

/** The accent dot: the item became idle or needs you since anyone last looked. */
export function UnseenDot({ className }: { className?: string }) {
  return <span role="img" aria-label="unseen" className={cn('inline-block size-1.5 shrink-0 rounded-full bg-primary', className)} />;
}

/** One state dot per item (named by its label, else its dot), plus the unseen dot. */
export function AgentDot({ state: s, unseenDot = true, className }: {
  state: AgentStateValue;
  /** Draw the unseen dot (default true); off where the label row already shows it. */
  unseenDot?: boolean;
  className?: string;
}) {
  return (
    <span className={cn('inline-flex shrink-0 items-center gap-1', className)}>
      <StatusDot status={s.dot} pulse={s.pulse} label={s.label ?? s.dot} />
      {unseenDot && s.unseen && <UnseenDot />}
    </span>
  );
}

const TONE: Record<AgentTone, string> = {
  running: 'text-running',
  waiting: 'text-waiting font-semibold',
  failed: 'text-failed',
  muted: 'text-muted-foreground',
};

/** The state word in its tone ("Needs you" in amber); nothing when the state has no label. */
export function AgentLabel({ state: s, className }: { state: AgentStateValue; className?: string }) {
  if (!s.label) return null;
  return <span className={cn('text-meta', TONE[s.tone], className)}>{s.label}</span>;
}
