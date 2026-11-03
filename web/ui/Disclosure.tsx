import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { Button, Disclosure as DisclosurePrimitive, DisclosurePanel } from 'react-aria-components';
import { cn } from '@ui/lib/utils';

export interface DisclosureProps {
  /** What the trigger says ("Advanced", "Thinking"); text or a richer line (an icon, a status). */
  title: ReactNode;
  /** A muted one-line summary after the title, cut with an ellipsis: what's inside, while it's closed. A `card` stacks
   *  it under the title, wrapping. */
  summary?: ReactNode;
  /** `card`: before the title (a status circle). */
  leading?: ReactNode;
  /** `card`: under the trigger whether open or not, above what folds away (a step's error and its actions). */
  visible?: ReactNode;
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
   * `card`: a bordered card whose header is the trigger (leading slot, the title over its summary, trailing, the
   * chevron at its end) and whose body folds (a setup step, a run's step on a phone).
   */
  variant?: 'inline' | 'row' | 'card';
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
  leading,
  visible,
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
  const card = variant === 'card';
  const chevron = (isExpanded: boolean) => (
    <ChevronRight
      aria-hidden
      className={cn(
        'shrink-0 text-muted-foreground transition-transform duration-(--pdt-duration) motion-reduce:transition-none',
        card ? 'size-4' : 'size-3.5',
        isExpanded && 'rotate-90',
      )}
    />
  );
  return (
    <DisclosurePrimitive
      isExpanded={open}
      defaultExpanded={defaultOpen}
      onExpandedChange={onOpenChange}
      isDisabled={disabled}
      className={cn(
        'flex min-w-0 flex-col',
        !row && !card && 'text-xs text-muted-foreground',
        card && 'overflow-hidden rounded-lg border border-border bg-card text-sm',
        className,
      )}
    >
      {({ isExpanded }) => (
        <>
          <Button
            slot="trigger"
            aria-label={ariaLabel}
            className={cn(
              'flex min-w-0 items-center text-left outline-none select-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50',
              card ? 'w-full gap-3 px-3.5 py-3 focus-visible:ring-inset pointer-coarse:min-h-control-touch' : 'gap-1.5 rounded-md',
              row && 'min-h-control-md w-full px-2 text-sm hover:bg-muted pointer-coarse:min-h-control-touch',
              !row && !card && 'w-fit max-w-full cursor-pointer hover:text-foreground',
              triggerClassName,
            )}
          >
            {card ? (
              <>
                {leading}
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="font-semibold">{title}</span>
                  {summary && <span className="text-xs text-muted-foreground wrap-anywhere">{summary}</span>}
                </span>
                {trailing && <span className="flex shrink-0 items-center gap-1.5">{trailing}</span>}
                {chevron(isExpanded)}
              </>
            ) : (
              <>
                {chevron(isExpanded)}
                <span className={cn('min-w-0', row && 'truncate font-medium', row && !summary && 'flex-1', row && summary && 'shrink-0')}>{title}</span>
                {summary && <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{summary}</span>}
                {trailing && <span className={cn('flex shrink-0 items-center gap-1.5', !row && !summary && 'ml-1')}>{trailing}</span>}
              </>
            )}
          </Button>
          {card && visible}
          <DisclosurePanel className={cn(isExpanded && (card ? 'px-3.5 pb-3.5' : row ? 'pt-1' : 'mt-1'), panelClassName)}>
            {isExpanded && children}
          </DisclosurePanel>
        </>
      )}
    </DisclosurePrimitive>
  );
}
