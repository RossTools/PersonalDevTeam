import type { Status } from '@ui';
import type { Session } from '../api/gen/Session';
import type { SessionStatus } from '../api/gen/SessionStatus';
import { agentState } from '../common/AgentState';

export const INITIAL_SIZE = { cols: 120, rows: 32 } as const;

/** A session's dot by its status alone (running green; ended or interrupted hollow), from common/AgentState. */
export function toDot(status: SessionStatus): Status {
  return agentState({ kind: 'shell', status }).dot;
}

/**
 * A session a workflow step started (a Coder, Reviewer, QA, Triage or PM step), with or without a ticket, running or
 * ended. The Agents list leaves these out even when a tab holds them: it lists the user's own conversations (PDT-299).
 * Sessions the user started, from a ticket's Start or not, have no step.
 */
export function startedByStep(s: Pick<Session, 'persona'>): boolean {
  return !!s.persona?.stepId;
}
