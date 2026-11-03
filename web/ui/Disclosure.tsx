import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { Button, Disclosure as DisclosurePrimitive, DisclosurePanel } from 'react-aria-components';
import { cn } from '@ui/lib/utils';

export interface DisclosureProps {
  /** What the trigger says ("Advanced", "Thinking"); text or a richer line (an icon, a status). */
  title: ReactNode;
  /** A muted one-line summary after the title, cut with an ellipsis: what's inside, while it's closed. */
  summary?: ReactNode;
  /** At the trigger's end (a badge). */
  trailing?: ReactNode;
  children: ReactNode;
  /** Controlled; leave out for a disclosure that keeps its own state. */
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /**
   * `inline` (default): a muted line of text that opens what's under it ("Show the comments"), in the surrounding
   * text size. `row`: a full-width row of `text-sm`, 44px on touch screens (a phone's folding section, a nav group).
   */
  variant?: 'inline' | 'row';
  disabled?: boolean;
  className?: string;
  /** On the trigger button (padding to line up with a card's rows). */
  triggerClassName?: string;
  /** On the panel under it. */
  panelClassName?: string;
  /** Names the trigger, and so the panel (a `group` named by its trigger), when it shows more than its name (a key). */
  'aria-label'?: string;
}

/**
 * Something folded away under a trigger: a chevron that turns when open, then the title. Enter/Space or a click
 * toggles it; the trigger says `aria-expanded` and controls the panel. Closed, its content isn't rendered.
 */
export function Disclosure({
  title,
  summary,
  trailing,
  children,
  open,
  defaultOpen,
  onOpenChange,
  variant = 'inline',
  disabled,
  className,
  triggerClassName,
  panelClassName,
  'aria-label': ariaLabel,
}: DisclosureProps) {
  const row = variant === 'row';
  return (
    <DisclosurePrimitive
      isExpanded={open}
      defaultExpanded={defaultOpen}
      onExpandedChange={onOpenChange}
      isDisabled={disabled}
      className={cn('flex min-w-0 flex-col', !row && 'text-xs text-muted-foreground', className)}
    >
      {({ isExpanded }) => (
        <>
          <Button
            slot="trigger"
            aria-label={ariaLabel}
            className={cn(
              'flex min-w-0 items-center gap-1.5 rounded-md text-left outline-none select-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50',
              row
                ? 'min-h-control-md w-full px-2 text-sm hover:bg-muted pointer-coarse:min-h-control-touch'
                : 'w-fit max-w-full cursor-pointer hover:text-foreground',
              triggerClassName,
            )}
          >
            <ChevronRight
              aria-hidden
              className={cn(
                'size-3.5 shrink-0 text-muted-foreground transition-transform duration-(--pdt-duration) motion-reduce:transition-none',
                isExpanded && 'rotate-90',
              )}
            />
            <span className={cn('min-w-0', row && 'truncate font-medium', row && !summary && 'flex-1', row && summary && 'shrink-0')}>{title}</span>
            {summary && <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{summary}</span>}
            {trailing && <span className={cn('flex shrink-0 items-center gap-1.5', !row && !summary && 'ml-1')}>{trailing}</span>}
          </Button>
          <DisclosurePanel className={cn(isExpanded && (row ? 'pt-1' : 'mt-1'), panelClassName)}>
            {isExpanded && children}
          </DisclosurePanel>
        </>
      )}
    </DisclosurePrimitive>
  );
}
