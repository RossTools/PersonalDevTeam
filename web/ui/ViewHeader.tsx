import type { ReactNode } from 'react';
import { cn } from './lib/utils';

export interface ViewHeaderProps {
  /** The view's title (truncates; the full text goes in `titleTooltip`). */
  title: ReactNode;
  /** Shown on hover over the title when it may be cut off. */
  titleTooltip?: string;
  /** Before the title: the status (a dot or an agent state). */
  leading?: ReactNode;
  /** After the title: chips and quiet meta text (branch, path, state, links). */
  meta?: ReactNode;
  /** Right-aligned: the primary action, then the ⋯ menu trigger. */
  actions?: ReactNode;
  /**
   * Pad the sides with the page gutter (`px-gutter`) instead of the tight 12px, for a view whose content sits on the
   * gutter (a ticket, an Inbox update), so header and content line up. Terminals, lists and boards keep it off.
   */
  gutter?: boolean;
  /** Rarely needed: the title is the header's heading. */
  'aria-label'?: string;
  className?: string;
}

/**
 * The header of every view: [status] title · chips/meta ··· actions. One line on desktop; on a narrow screen the
 * actions wrap to a second line. A fixed minimum height keeps the content below (the terminal) from jumping.
 */
export function ViewHeader({ title, titleTooltip, leading, meta, actions, gutter = false, className, ...rest }: ViewHeaderProps) {
  return (
    <header
      {...rest}
      className={cn(
        'flex min-h-10 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border py-1.5 text-sm',
        gutter ? 'px-gutter' : 'px-3',
        className,
      )}
    >
      <div className="flex min-w-0 flex-[1_1_12rem] items-center gap-2">
        {leading != null && <span className="flex shrink-0 items-center">{leading}</span>}
        <h2 className="min-w-0 max-w-[60%] shrink-0 truncate font-medium" title={titleTooltip}>{title}</h2>
        {meta != null && (
          <div className="flex min-w-0 flex-1 items-center gap-2 truncate text-xs text-muted-foreground">{meta}</div>
        )}
      </div>
      {actions != null && <div className="ml-auto flex shrink-0 items-center gap-1">{actions}</div>}
    </header>
  );
}
