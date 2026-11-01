import type { ReactNode } from 'react';
import { cn } from './lib/utils';

/**
 * A command or a tool call in a card: a header (what runs, and where, on the left; why, on the right) over the code,
 * kept as written. `children` replaces the code with something else (a diff).
 */
export function CodeCard({ title, aside, code, children, className, 'aria-label': ariaLabel }: {
  title?: ReactNode;
  aside?: ReactNode;
  code?: string;
  children?: ReactNode;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <div role="group" aria-label={ariaLabel} className={cn('overflow-hidden rounded-lg border border-border bg-card', className)}>
      {(title || aside) && (
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-0.5 border-b border-border px-3.5 py-2 text-xs text-muted-foreground">
          {title && <span className="min-w-0 break-words">{title}</span>}
          {aside && <span className="min-w-0 break-words">{aside}</span>}
        </div>
      )}
      {code !== undefined && (
        <pre className="max-h-96 overflow-auto px-3.5 py-3 font-mono text-xs whitespace-pre-wrap break-words">{code}</pre>
      )}
      {children}
    </div>
  );
}
