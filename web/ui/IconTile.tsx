import type { ReactNode } from 'react';
import { cn } from './lib/utils';

export type IconTileTone = 'neutral' | 'accent' | 'running' | 'waiting' | 'failed';

const TONE: Record<IconTileTone, string> = {
  neutral: 'bg-muted text-muted-foreground',
  accent: 'bg-accent text-link',
  running: 'bg-running-soft text-running',
  waiting: 'bg-waiting-soft text-waiting',
  failed: 'bg-failed-soft text-failed',
};

const SIZE = {
  sm: 'size-4.5 rounded-sm [&>svg]:size-3',
  md: 'size-7 rounded-md [&>svg]:size-3.5',
};

/**
 * An icon on a small tinted square: a list row's leading icon (`sm`, an Inbox item's kind), a picker's (`md`, a
 * file's kind). A tone only when the colour shows state or kind; `neutral` otherwise.
 */
export function IconTile({ tone = 'neutral', size = 'md', children, className, 'aria-hidden': hidden }: {
  tone?: IconTileTone;
  size?: 'sm' | 'md';
  /** The icon. Give it an `aria-label`, or hide the tile (`aria-hidden`) when the row already says it. */
  children: ReactNode;
  className?: string;
  'aria-hidden'?: boolean;
}) {
  return (
    <span aria-hidden={hidden} data-slot="icon-tile" className={cn('grid shrink-0 place-items-center', SIZE[size], TONE[tone], className)}>
      {children}
    </span>
  );
}
