import type { Status } from '@ui';
import type { SessionStatus } from '../api/gen/SessionStatus';

export const INITIAL_SIZE = { cols: 120, rows: 32 } as const;

export function toDot(status: SessionStatus): Status {
  switch (status) {
    case 'running': return 'running';
    case 'interrupted': return 'waiting';
    case 'finished': return 'idle';
  }
}
