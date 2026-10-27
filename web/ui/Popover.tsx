import type { ReactElement, ReactNode, RefObject } from 'react';
import { cn } from '@ui/lib/utils';
import { Dialog as AriaDialog, Heading } from 'react-aria-components';
import { Popover as PopoverPrimitive, PopoverTrigger } from './primitives/popover';
import { pressableChild } from './lib/pressableChild';

export type PopoverPlacement = 'bottom' | 'bottom start' | 'bottom end' | 'top' | 'top start' | 'top end' | 'left' | 'right';

export interface PopoverProps {
  /** The element that opens it: a Button (or a DOM button). Focus returns to it on close. */
  trigger?: ReactElement;
  /** Instead of `trigger`: an element elsewhere to anchor to (control it with `open`/`onOpenChange`). Focus returns to
   * whatever had it when the panel opened. */
  anchor?: RefObject<Element | null>;
  /** Accessible name of the panel when there's no visible `title`. */
  label?: string;
  /** A visible heading at the top of the panel (also its accessible name). */
  title?: ReactNode;
  /** Content; a function gets `close` (a picker closes itself after a choice). */
  children: ReactNode | ((opts: { close: () => void }) => ReactNode);
  placement?: PopoverPlacement;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Width and padding overrides (default w-72, p-2.5); `p-0` suits a CommandList. */
  className?: string;
}

/**
 * An anchored panel (filters, pickers). Click outside or Escape closes it, focus returns to the trigger. Draws above
 * dialogs and sheets (z-(--pdt-z-menu)), so a picker inside a dialog works. It keeps to the height react-aria allows
 * (less when a phone's keyboard is open) and scrolls inside it, like a Menu.
 */
export function Popover({ trigger, anchor, label, title, children, placement = 'bottom start', open, onOpenChange, className }: PopoverProps) {
  const panel = (
    <AriaDialog aria-label={title ? undefined : label} className="flex min-h-0 flex-col gap-2 overflow-y-auto outline-none">
      {({ close }) => (
        <>
          {title && (
            <Heading slot="title" className="text-xs font-semibold">
              {title}
            </Heading>
          )}
          {typeof children === 'function' ? children({ close }) : children}
        </>
      )}
    </AriaDialog>
  );
  if (!trigger) {
    return (
      <PopoverPrimitive
        triggerRef={anchor}
        isOpen={open ?? false}
        onOpenChange={onOpenChange}
        placement={placement}
        className={cn('max-w-[calc(100vw-2rem)]', className)}
      >
        {panel}
      </PopoverPrimitive>
    );
  }
  return (
    <PopoverTrigger {...(open === undefined ? {} : { isOpen: open })} onOpenChange={onOpenChange}>
      {pressableChild(trigger)}
      <PopoverPrimitive placement={placement} className={cn('max-w-[calc(100vw-2rem)]', className)}>
        {panel}
      </PopoverPrimitive>
    </PopoverTrigger>
  );
}
