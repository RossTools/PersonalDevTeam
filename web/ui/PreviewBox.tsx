import type { ReactNode } from 'react';
import { cn } from './lib/utils';

/**
 * A quiet inset box for what a request carries (the command it runs, what it would create), inside a card.
 * `as="pre"` keeps text as written.
 */
export function PreviewBox({ as: Tag = 'div', children, className }: { as?: 'div' | 'pre'; children: ReactNode; className?: string }) {
  return <Tag data-slot="preview-box" className={cn('rounded-md border border-border bg-surface-2 p-2.5', className)}>{children}</Tag>;
}
