import type { AriaRole, ReactNode } from 'react';
import { cn } from 'cn';

export type BannerTone = 'neutral' | 'accent' | 'running' | 'waiting' | 'failed';

const tones: Record<BannerTone, string> = {
  neutral: 'bg-background text-foreground',
  accent: 'bg-accent text-foreground',
  running: 'bg-running-soft text-running',
  waiting: 'bg-waiting-soft text-waiting',
  failed: 'bg-failed-soft text-failed',
};

const defaultRole: Partial<Record<BannerTone, AriaRole>> = { running: 'status', waiting: 'status', failed: 'alert' };

/**
 * A host-wide notice: one line of fixed height (h-8) that never wraps. It sits above the terminal, so any change in
 * its height would resize the PTY (spec §3.4); long text truncates (pass `title` for the full text). Anything that
 * needs more room floats over the page from inside `actions` (e.g. a popover).
 */
export function Banner({
  tone = 'neutral',
  children,
  actions,
  title,
  role,
  className,
}: {
  tone?: BannerTone;
  children: ReactNode;
  actions?: ReactNode;
  title?: string;
  role?: AriaRole;
  className?: string;
}) {
  return (
    <div
      role={role ?? defaultRole[tone]}
      data-tone={tone}
      className={cn(
        'relative flex h-8 shrink-0 flex-nowrap items-center gap-2 overflow-hidden border-b border-border px-3 text-xs whitespace-nowrap',
        tones[tone],
        className,
      )}
    >
      <span className="min-w-0 truncate" title={title}>
        {children}
      </span>
      {actions && <span className="ml-auto flex shrink-0 flex-nowrap items-center gap-1">{actions}</span>}
    </div>
  );
}
