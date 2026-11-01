import { cn } from '@ui/lib/utils';

/**
 * running = working (green), waiting = needs you (amber), failed (red), idle (grey), ended = hollow grey ring,
 * background = a hollow green ring (waiting on background work; pulses slowly).
 */
export type Status = 'running' | 'waiting' | 'failed' | 'idle' | 'ended' | 'background';

const color: Record<Status, string> = {
  running: 'bg-running',
  waiting: 'bg-waiting',
  failed: 'bg-failed',
  idle: 'bg-idle',
  ended: 'border border-idle bg-transparent',
  background: 'border-2 border-running bg-transparent',
};

/**
 * The state dot. `pulse` applies to `running` only (an agent working, spec §6.5); `background` always pulses, slowly.
 * Neither pulses under reduced motion. `label` overrides the accessible name (defaults to the status).
 */
export function StatusDot({
  status,
  pulse = false,
  label,
  className,
}: {
  status: Status;
  pulse?: boolean;
  label?: string;
  className?: string;
}) {
  return (
    <span
      role="img"
      aria-label={label ?? status}
      data-status={status}
      data-pulse={pulse && status === 'running' ? '' : undefined}
      className={cn(
        'inline-block size-2 shrink-0 rounded-full',
        color[status],
        pulse && status === 'running' && 'animate-pulse motion-reduce:animate-none',
        status === 'background' && 'animate-pulse-slow motion-reduce:animate-none',
        className,
      )}
    />
  );
}
