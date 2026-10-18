import type { Status } from '@ui';
import type { SessionStatus } from '../api/gen/SessionStatus';
import { agentState } from '../common/AgentState';

export const INITIAL_SIZE = { cols: 120, rows: 32 } as const;

/** A session's dot by its status alone (running green; ended or interrupted hollow), from common/AgentState. */
export function toDot(status: SessionStatus): Status {
  return agentState({ kind: 'shell', status }).dot;
}
