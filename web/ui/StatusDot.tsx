export type Status = 'running' | 'waiting' | 'failed' | 'idle';

const color: Record<Status, string> = {
  running: 'bg-running',
  waiting: 'bg-waiting',
  failed: 'bg-failed',
  idle: 'bg-idle',
};

export function StatusDot({ status }: { status: Status }) {
  return <span role="img" aria-label={status} className={`inline-block size-2 rounded-full ${color[status]}`} />;
}
