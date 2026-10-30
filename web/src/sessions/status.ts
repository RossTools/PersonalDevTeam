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
 * A session a workflow step started that the session lists leave out unless a tab holds it: one for a ticket, running
 * or ended (PDT-290; it opens from the ticket's Sessions), and one with no ticket (Triage, a PM step) once it has ended
 * (PDT-263). Both open from their run's step. Sessions the user started are always listed.
 */
export function hiddenStepSession(s: Pick<Session, 'persona' | 'attemptId' | 'ticket' | 'status'>): boolean {
  if (!s.persona?.stepId) return false;
  return !!s.attemptId || !!s.ticket || s.status !== 'running';
}
