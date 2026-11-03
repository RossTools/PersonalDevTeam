import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';

/**
 * Where something comes from, on a light band: its icon and name, the account or place under the name, and its action
 * (Browse Drive…) at the end. Below `sm` the action wraps under the name.
 */
export function SourceCard({ icon, name, detail, action, children, className, 'aria-label': ariaLabel }: {
  icon?: ReactNode;
  name: ReactNode;
  /** One muted line under the name: the signed-in account, "2 accounts", a folder. */
  detail?: ReactNode;
  /** A button or menu button at the end. */
  action?: ReactNode;
  /** Under the row, full width: setup help. */
  children?: ReactNode;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <div
      data-slot="source-card"
      role={ariaLabel ? 'group' : undefined}
      aria-label={ariaLabel}
      className={cn('flex flex-col gap-2 rounded-lg border border-border bg-band px-3 py-2.5', className)}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        {icon && <span aria-hidden className="flex shrink-0 text-muted-foreground [&_svg]:size-5">{icon}</span>}
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium text-foreground">{name}</span>
          {detail && <span className="truncate text-meta text-muted-foreground">{detail}</span>}
        </div>
        {action && <div className="flex shrink-0 max-sm:w-full max-sm:*:w-full">{action}</div>}
      </div>
      {children}
    </div>
  );
}
