import type { ReactNode } from 'react';
import { cn } from './lib/utils';

export type CalloutTone = 'error' | 'warning' | 'success' | 'info';

const tones: Record<CalloutTone, string> = {
  error: 'border-l-failed bg-linear-to-r from-failed-soft to-transparent to-60%',
  warning: 'border-l-waiting bg-linear-to-r from-waiting-soft to-transparent to-60%',
  success: 'border-l-running bg-linear-to-r from-running-soft to-transparent to-60%',
  info: 'border-l-input',
};

/**
 * A notice in the page's flow, coloured by severity down its left edge: what failed, what is running low. `title` is
 * one bold line over the body. It is in the page from the start, so it announces nothing (an `Alert` is for news).
 */
export function Callout({ tone = 'info', title, children, className, 'aria-label': ariaLabel }: {
  tone?: CalloutTone;
  title?: ReactNode;
  children?: ReactNode;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <div
      role={ariaLabel ? 'region' : undefined}
      aria-label={ariaLabel}
      data-tone={tone}
      className={cn('rounded-lg border border-l-3 border-border bg-card px-3.5 py-3 text-sm text-foreground', tones[tone], className)}
    >
      {title && <div className="mb-1 font-semibold">{title}</div>}
      {children}
    </div>
  );
}
