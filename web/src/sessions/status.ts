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
 * A workflow step's session with no ticket (Triage, a PM step) that has ended: the session lists leave it out
 * (PDT-263) unless a tab holds it. It stays, and opens from its run's step.
 */
export function hiddenStepSession(s: Pick<Session, 'persona' | 'attemptId' | 'ticket' | 'status'>): boolean {
  return !!s.persona?.stepId && !s.attemptId && !s.ticket && s.status !== 'running';
}
