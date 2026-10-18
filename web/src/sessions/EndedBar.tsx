import { Button } from '@ui';
import { cn } from '@ui/lib/utils';

export interface EndedBarProps {
  /** `finished`: stopped or exited. `interrupted`: it was running when the daemon stopped. */
  status: 'finished' | 'interrupted';
  /** An agent session: resuming continues its last conversation. */
  agent?: boolean;
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
 * size: "Session ended · Restart · Remove…" or "Interrupted when the daemon stopped · Resume".
 */
export function EndedBar({ status, agent = false, onRestart, onRemove, restarting, removing, disabled, className }: EndedBarProps) {
  const interrupted = status === 'interrupted';
  const message = interrupted ? 'Interrupted when the daemon stopped' : 'Session ended';
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
        <span className="shrink-0 font-medium">{message}</span>
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
