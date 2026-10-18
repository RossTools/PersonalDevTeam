import type { HTMLAttributes, ReactNode } from 'react';
import { cn } from 'cn';
import { Badge as BadgePrimitive } from './primitives/badge';

export type BadgeTone = 'neutral' | 'running' | 'waiting' | 'failed' | 'accent';

/**
 * A small label. Neutral by default; a tone only when it shows state (colour means state, spec §3).
 * `count` renders a round number badge (capped at `max`, 99 by default) instead of `children`.
 */
export function Badge({
  tone = 'neutral',
  count,
  max = 99,
  children,
  className,
  ...rest
}: Omit<HTMLAttributes<HTMLSpanElement>, 'children'> & {
  tone?: BadgeTone;
  count?: number;
  max?: number;
  children?: ReactNode;
}) {
  const isCount = count !== undefined;
  return (
    <BadgePrimitive variant={tone} shape={isCount ? 'count' : 'label'} className={cn(className)} {...rest}>
      {isCount ? (count > max ? `${max}+` : count) : children}
    </BadgePrimitive>
  );
}
