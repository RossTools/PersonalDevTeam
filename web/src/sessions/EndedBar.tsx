import { Button, RelativeTime } from '@ui';
import { cn } from '@ui/lib/utils';

export interface EndedBarProps {
  /** `finished`: stopped or exited. `interrupted`: it was running when the daemon stopped. */
  status: 'finished' | 'interrupted';
  /** An agent session: resuming continues its last conversation. */
  agent?: boolean;
  /** When it ended (ms), and its exit code (finished only; null when it was stopped or the code is unknown). */
  endedAt?: number | null;
  exitCode?: number | null;
  /** Restart (finished) or Resume (interrupted). */
  onRestart: () => void;
  /** Remove… (finished only); asks first, in the caller. */
  onRemove?: () => void;
  restarting?: boolean;
  removing?: boolean;
  /** Another action is in flight. */
  disabled?: boolean;
  className?: string;
}

/**
 * The fixed-height bar floated over the bottom edge of an ended session's (dimmed) terminal, so the terminal keeps its
 * size: "Ended 2h ago · exit 0 · Restart · Remove…" or "Interrupted when the daemon stopped · Resume".
 */
export function EndedBar({ status, agent = false, endedAt = null, exitCode = null, onRestart, onRemove, restarting, removing, disabled, className }: EndedBarProps) {
  const interrupted = status === 'interrupted';
  const message = interrupted ? 'Interrupted when the daemon stopped' : 'Session ended';
  const exit = !interrupted && exitCode !== null && exitCode >= 0 ? `exit ${exitCode}` : null;
  return (
    <div
      role="group"
      aria-label={message}
      className={cn(
        'absolute inset-x-0 bottom-0 flex h-12 items-center gap-2 overflow-hidden border-t border-border bg-surface px-3 text-sm shadow-popover',
        className,
      )}
    >
      <div className="flex min-w-0 flex-1 items-baseline gap-2">
        {!interrupted && endedAt !== null ? (
          <span className="shrink-0 font-medium">Ended <RelativeTime time={endedAt} /></span>
        ) : (
          <span className="shrink-0 font-medium">{message}</span>
        )}
        {exit && <span className="shrink-0 text-meta text-muted-foreground">· {exit}</span>}
        {interrupted && agent && (
          <span className="min-w-0 truncate text-meta text-muted-foreground">the agent resumes its last conversation</span>
        )}
      </div>
      <Button variant="primary" size="sm" compact disabled={disabled} pending={restarting} onClick={onRestart}>
        {interrupted ? 'Resume' : 'Restart'}
      </Button>
      {!interrupted && onRemove && (
        <Button variant="ghost" size="sm" compact disabled={disabled} pending={removing} onClick={onRemove}>
          Remove…
        </Button>
      )}
    </div>
  );
}
