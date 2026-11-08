import type { AriaRole, ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { Empty, EmptyContent, EmptyDescription, EmptyMedia } from './primitives/empty';

/**
 * An empty list or page: an icon, one sentence and the next step as `action`. Also the page-load error
 * (the message plus a Retry action). `size="inline"` is the one muted line in a section or card ("None yet."); with
 * `tone="error"`, the line that says why it is empty ("Can't load the inbox.", a call's failure), announced.
 */
export function EmptyState({
  icon,
  children,
  action,
  size = 'page',
  tone = 'muted',
  role,
  className,
}: {
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  /** `page` (default): centred, with room around it. `inline`: a muted `text-xs` line, left-aligned, where a list would be. */
  size?: 'page' | 'inline';
  /** `inline` only: `error` colours the line and makes it `role=alert` (unless `role` says otherwise). */
  tone?: 'muted' | 'error';
  role?: AriaRole;
  className?: string;
}) {
  if (size === 'inline') {
    return (
      <p
        data-slot="empty"
        role={role ?? (tone === 'error' ? 'alert' : undefined)}
        className={cn('flex flex-wrap items-center gap-x-2 text-xs break-words', tone === 'error' ? 'text-failed' : 'text-muted-foreground', className)}
      >
        {icon && <span aria-hidden className="flex [&_svg]:size-3.5">{icon}</span>}
        {children}
        {action}
      </p>
    );
  }
  return (
    <Empty role={role} className={cn(className)}>
      {icon && (
        <EmptyMedia variant="icon" aria-hidden>
          {icon}
        </EmptyMedia>
      )}
      <EmptyDescription>{children}</EmptyDescription>
      {action && <EmptyContent>{action}</EmptyContent>}
    </Empty>
  );
}
