import type { Status } from '@ui';
import type { RunState } from '../api/gen/RunState';

export function runDot(state: RunState): Status {
  switch (state) {
    case 'setting_up':
    case 'starting': return 'waiting';
    case 'ready':
    case 'running':
    case 'passed': return 'running';
    case 'failed': return 'failed';
    case 'stopped': return 'idle';
  }
}

export const runLabel = (state: RunState) => state.replace('_', ' ');
export const isActive = (state: RunState) => ['setting_up', 'starting', 'ready', 'running'].includes(state);
