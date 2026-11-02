import type { ReactElement, ReactNode } from 'react';
import { Focusable, Tooltip as AriaTooltip, TooltipTrigger } from 'react-aria-components';
import { cn } from './lib/utils';

export interface HoverCardProps {
  /** The trigger: a focusable element with an interactive role (a link) that forwards its ref. */
  children: ReactElement;
  /** What the card shows: a short summary to read, nothing to press (it closes when the pointer leaves). */
  content: ReactNode;
  /** Called as it opens and closes (fetch what it shows on open). */
  onOpenChange?: (open: boolean) => void;
  /** Hover delay in ms before it opens (keyboard focus opens it at once). */
  delay?: number;
  placement?: 'top' | 'bottom' | 'top start' | 'bottom start';
  className?: string;
}

/**
 * A card about the thing under the pointer (a ticket key's title, stage and PR): it opens a moment after the pointer
 * arrives, or at once on keyboard focus, and closes when the pointer leaves, on blur or on Escape. It describes its
 * trigger (`aria-describedby`), so it holds text only.
 */
export function HoverCard({ children, content, onOpenChange, delay = 400, placement = 'bottom start', className }: HoverCardProps) {
  return (
    <TooltipTrigger delay={delay} closeDelay={150} onOpenChange={onOpenChange}>
      <Focusable>{children as Parameters<typeof Focusable>[0]['children']}</Focusable>
      <AriaTooltip
        placement={placement}
        offset={6}
        className={cn(
          'z-(--pdt-z-menu) flex w-72 max-w-[calc(100vw-2rem)] flex-col gap-1.5 rounded-lg border border-border bg-popover p-3 text-xs text-foreground shadow-popover duration-(--pdt-duration) motion-reduce:animate-none data-entering:animate-in data-entering:fade-in-0 data-exiting:animate-out data-exiting:fade-out-0',
          className,
        )}
      >
        {content}
      </AriaTooltip>
    </TooltipTrigger>
  );
}
