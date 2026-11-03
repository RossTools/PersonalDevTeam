import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';

type HeadingTag = 'h2' | 'h3' | 'h4' | 'div' | 'span' | 'legend';

const label = 'text-2xs font-medium tracking-wider text-muted-foreground uppercase';

export interface SectionLabelProps {
  children: ReactNode;
  /** The element: a heading level that fits the page's outline (h3 by default), `legend` for a fieldset, or `div`/`span` when it heads nothing. */
  as?: HeadingTag;
  id?: string;
  className?: string;
}

/** The small uppercase label over a group of things: a panel's section ("Links", "Parameters"), a dialog's group. */
export function SectionLabel({ children, as: Tag = 'h3', id, className }: SectionLabelProps) {
  return <Tag id={id} className={cn(label, className)}>{children}</Tag>;
}

/**
 * A list's group heading in a side list (Inbox, Tickets, Notes): a `SectionLabel` inset to line up with the rows
 * (`px-3`; `className="px-2"` in a narrower rail). `trailing` controls (a ＋ button, Mark all read) sit at its right,
 * beside the heading rather than in it, so they stay out of its name.
 */
export function ListHeading({ children, trailing, as: Tag = 'h2', id, className }: SectionLabelProps & { trailing?: ReactNode }) {
  const inset = 'flex items-center gap-1.5 px-3 pt-3 pb-1';
  if (!trailing) return <Tag id={id} className={cn(inset, label, className)}>{children}</Tag>;
  return (
    <div className={cn(inset, className)}>
      <Tag id={id} className={cn('flex min-w-0 flex-1 items-center gap-1.5', label)}>{children}</Tag>
      {trailing}
    </div>
  );
}
