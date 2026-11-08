import { Children, type HTMLAttributes, type ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { Badge as BadgePrimitive } from './primitives/badge';

export type BadgeTone = 'neutral' | 'running' | 'waiting' | 'failed' | 'accent' | 'solid';

/** Each run of text in a label in a span that truncates with an ellipsis, so a long badge never widens its container. */
function truncatingText(children: ReactNode): ReactNode[] {
  const out: ReactNode[] = [];
  let text: string[] = [];
  const flush = () => {
    const run = text.join('');
    text = [];
    // A space between an icon and a span is no flex item (it collapses away); keep it that way.
    if (run.trim() === '') return;
    out.push(<span key={`t${out.length}`} className="min-w-0 truncate">{run}</span>);
  };
  for (const child of Children.toArray(children)) {
    if (typeof child === 'string' || typeof child === 'number') text.push(String(child));
    else {
      flush();
      out.push(child);
    }
  }
  flush();
  return out;
}

/**
 * A small label. Neutral by default; a tone only when it shows state (colour means state).
 * `count` renders a round number badge (capped at `max`, 99 by default) instead of `children`. `tone="solid"` is the
 * accent-filled count of things waiting on the user (the rail's Needs you); `size="sm"` (16px) sits on an icon or in a
 * nav row.
 * `pill` rounds it and sets it bold (a kind over an item: "? Question").
 * A label is never wider than its container: its text truncates, so give a long one a `title` with the full text.
 */
export function Badge({
  tone = 'neutral',
  count,
  max = 99,
  size = 'md',
  pill = false,
  children,
  className,
  ...rest
}: Omit<HTMLAttributes<HTMLSpanElement>, 'children'> & {
  tone?: BadgeTone;
  pill?: boolean;
  count?: number;
  max?: number;
  size?: 'sm' | 'md';
  children?: ReactNode;
}) {
  const isCount = count !== undefined;
  return (
    <BadgePrimitive variant={tone} shape={isCount ? 'count' : 'label'} size={size} className={cn(!isCount && 'max-w-full min-w-0 justify-start', pill && 'rounded-full font-semibold', className)} {...rest}>
      {isCount ? (count > max ? `${max}+` : count) : truncatingText(children)}
    </BadgePrimitive>
  );
}
