import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { CircleAlertIcon, InfoIcon, TriangleAlertIcon } from 'lucide-react';
import { Alert as AlertPrimitive, AlertAction, AlertDescription, AlertTitle } from './primitives/alert';

export type AlertTone = 'error' | 'warning' | 'info';

const icons: Record<AlertTone, ReactNode> = {
  error: <CircleAlertIcon aria-hidden />,
  warning: <TriangleAlertIcon aria-hidden />,
  info: <InfoIcon aria-hidden />,
};

/**
 * An inline message (form errors, warnings, notes). Errors are `role=alert` (announced); the others are
 * `role=status`. `action` is a node (usually a small Button) on the right.
 */
export function Alert({
  tone = 'info',
  title,
  children,
  action,
  id,
  className,
  tabIndex,
}: {
  tone?: AlertTone;
  title?: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  id?: string;
  className?: string;
  /** -1 lets a form move focus to its error. */
  tabIndex?: number;
}) {
  return (
    <AlertPrimitive
      id={id}
      variant={tone}
      role={tone === 'error' ? 'alert' : 'status'}
      tabIndex={tabIndex}
      className={cn('outline-none focus-visible:ring-2 focus-visible:ring-ring', className)}
    >
      {icons[tone]}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {title && <AlertTitle>{title}</AlertTitle>}
        {children && <AlertDescription>{children}</AlertDescription>}
      </div>
      {action && <AlertAction>{action}</AlertAction>}
    </AlertPrimitive>
  );
}
