import type { ReactElement, ReactNode } from 'react';
import { Tooltip as TooltipContent, TooltipTrigger } from '@ui/primitives/tooltip';

export interface TooltipProps {
  /** What the tooltip says. */
  content: ReactNode;
  /** The trigger: one @ui control (Button, IconButton, Link). Set `wrap` for any other focusable element. */
  children: ReactElement;
  placement?: 'top' | 'bottom' | 'left' | 'right';
  /** Hover delay in ms before it opens (keyboard focus opens it at once). */
  delay?: number;
  /** Wrap a non-@ui trigger (it must forward its ref, be focusable and have an interactive role). */
  wrap?: boolean;
  isDisabled?: boolean;
}

/** A short label shown on hover and keyboard focus; closes on Escape, blur or pointer leave. */
export function Tooltip({ content, children, placement = 'top', delay, wrap = false, isDisabled }: TooltipProps) {
  return (
    <TooltipTrigger delay={delay} wrapTrigger={wrap} isDisabled={isDisabled}>
      {children}
      <TooltipContent placement={placement}>{content}</TooltipContent>
    </TooltipTrigger>
  );
}
