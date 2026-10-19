import type { ReactNode } from 'react';
import { cn } from './lib/utils';

export interface MessageBubbleProps {
  children: ReactNode;
  /** `pending`: written but not yet taken by the agent (dimmed, with `label`). */
  tone?: 'default' | 'pending';
  /** A small label above the text ("Queued", "Sending now"). */
  label?: ReactNode;
  /** A line under the bubble (where it came from, a time). */
  meta?: ReactNode;
  className?: string;
}

/**
 * The user's message in a conversation: right-aligned in `surface-2`, text kept as written (line breaks and runs of
 * spaces), at most ~85% of the row wide so it reads as a reply on phones too.
 */
export function MessageBubble({ children, tone = 'default', label, meta, className }: MessageBubbleProps) {
  return (
    <div className={cn('flex flex-col items-end gap-0.5', className)} data-tone={tone}>
      <div
        className={cn(
          'max-w-[85%] rounded-lg bg-surface-2 px-3 py-2 text-sm whitespace-pre-wrap break-words text-foreground',
          tone === 'pending' && 'border border-dashed border-border bg-transparent text-muted-foreground',
        )}
      >
        {label && <div className="mb-0.5 text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>}
        {children}
      </div>
      {meta && <div className="text-meta text-muted-foreground">{meta}</div>}
    </div>
  );
}
