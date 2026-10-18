import type { Status } from '@ui';
import type { RunState } from '../api/gen/RunState';
import { agentState } from '../common/AgentState';

/** The run's dot, from the one status vocabulary (common/AgentState). */
export function runDot(state: RunState): Status {
  return agentState({ kind: 'run', status: state }).dot;
}

export const runLabel = (state: RunState) => state.replace('_', ' ');
export const isActive = (state: RunState) => ['setting_up', 'starting', 'ready', 'running'].includes(state);
