import type { AriaRole, ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { Empty, EmptyContent, EmptyDescription, EmptyMedia } from './primitives/empty';

/**
 * An empty list or page: an icon, one sentence and the next step as `action` (spec §6.4). Also the page-load error
 * (the message plus a Retry action, §6.3).
 */
export function EmptyState({
  icon,
  children,
  action,
  role,
  className,
}: {
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  role?: AriaRole;
  className?: string;
}) {
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
