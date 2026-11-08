import type { ReactNode, Ref } from 'react';
import { cn } from './lib/utils';

/**
 * What an agent's tool did, as a card in a conversation: an icon and a heading, something on the right (a status, an
 * action), then its rows, divided. A `group`, not a landmark: a long conversation holds many.
 */
export function ToolCard({ icon, heading, aside, children, 'aria-label': ariaLabel, ref, className }: {
  icon: ReactNode;
  heading: ReactNode;
  aside?: ReactNode;
  /** The rows, each a child divided from the next. */
  children?: ReactNode;
  /** The card's name (the heading's text). */
  'aria-label': string;
  /** Given, the card can take focus (from a button of its own that went away). */
  ref?: Ref<HTMLDivElement>;
  className?: string;
}) {
  return (
    <div
      ref={ref}
      role="group"
      tabIndex={ref ? -1 : undefined}
      aria-label={ariaLabel}
      data-slot="tool-card"
      className={cn('rounded-lg border border-border bg-card text-sm shadow-card outline-none focus-visible:ring-2 focus-visible:ring-ring', className)}
    >
      <div className={cn('flex min-h-9 items-center gap-2 px-3 py-1.5 text-xs text-muted-foreground', children && 'border-b border-border')}>
        <span aria-hidden className="grid size-4 shrink-0 place-items-center text-link [&>svg]:size-3.5">{icon}</span>
        <span className="flex min-w-0 flex-wrap items-center gap-1.5">{heading}</span>
        <span className="flex-1" />
        {aside}
      </div>
      {children && <div className="flex flex-col divide-y divide-border px-3">{children}</div>}
    </div>
  );
}
