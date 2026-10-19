import { useState, type ReactElement, type ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { DialogTrigger } from 'react-aria-components';
import { XIcon } from 'lucide-react';
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from './primitives/drawer';
import { pressableChild } from './lib/pressableChild';

export type SheetSide = 'left' | 'right' | 'bottom';

const swipe = { left: 'left', right: 'right', bottom: 'down' } as const;
// A modal sheet's backdrop: the overlay, or the full-screen viewport above it that holds the sheet.
const isBackdrop = (t: EventTarget | null) =>
  t instanceof Element && t.matches('[data-slot="drawer-overlay"], [data-slot="drawer-viewport"]');

export interface SheetProps {
  side?: SheetSide;
  /** The sheet's heading and accessible name. */
  title: ReactNode;
  /** Keep the title for screen readers only (the phone sidebar has its own header). */
  hideTitle?: boolean;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  /** A Button (or DOM button) that opens it. Without one, control it with `open`/`onOpenChange`. */
  trigger?: ReactElement;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Default true: a backdrop, focus kept inside. False leaves the page beside it usable (the ticket peek on desktop:
   *  the backlog stays live next to it); Escape and the close button still close it. */
  modal?: boolean;
  className?: string;
}

/**
 * A side or bottom panel (phone sidebar, ticket peek). Opens from a button or `open`, never from an edge swipe (that's
 * the system Back gesture on phones). Closes on a swipe towards its edge, Escape, the close button or (modal only) a
 * tap on the backdrop; focus returns to what opened it.
 */
export function Sheet({
  modal = true,
  side = 'right',
  title,
  hideTitle,
  description,
  children,
  footer,
  trigger,
  open,
  onOpenChange,
  className,
}: SheetProps) {
  const [inner, setInner] = useState(false);
  const isOpen = open ?? inner;
  const setOpen = (v: boolean) => {
    if (open === undefined) setInner(v);
    onOpenChange?.(v);
  };
  const drawer = (
    // A non-modal sheet ignores presses and focus outside it: the page beside it is meant to be used, and its own
    // menus and dialogs render in portals outside it. A modal one closes on a press on its backdrop only: Base UI
    // also counts a click on any element around the sheet, which is where a click lands when a menu opened on
    // mouse down covers the button by mouse up (Ticket actions on a narrow window).
    <Drawer
      modal={modal}
      disablePointerDismissal={!modal}
      open={isOpen}
      onOpenChange={(v, details) => {
        if (details.reason === 'outside-press' && !isBackdrop(details.event.target)) {
          details.cancel();
          return;
        }
        setOpen(v);
      }}
      swipeDirection={swipe[side]}
      showSwipeHandle={side === 'bottom'}
    >
      <DrawerContent className={cn('bg-surface', className)}>
        <DrawerHeader className={cn('relative p-3 pr-10 text-left', hideTitle && 'sr-only')}>
          <DrawerTitle className="text-sm font-semibold">{title}</DrawerTitle>
          {description && <DrawerDescription className="text-xs">{description}</DrawerDescription>}
        </DrawerHeader>
        <DrawerClose
          aria-label="Close"
          className="absolute top-2 right-2 inline-flex size-control-md items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:size-control-touch"
        >
          <XIcon className="size-4" aria-hidden />
        </DrawerClose>
        <div className="min-h-0 flex-1 overflow-auto px-3 pb-3">{children}</div>
        {footer && <DrawerFooter className="flex-row justify-end border-t border-border p-3">{footer}</DrawerFooter>}
      </DrawerContent>
    </Drawer>
  );
  if (!trigger) return drawer;
  // DialogTrigger only lends its press handling to the trigger (and aria-expanded); the drawer does the rest.
  return (
    <DialogTrigger isOpen={isOpen} onOpenChange={setOpen}>
      {pressableChild(trigger)}
      {drawer}
    </DialogTrigger>
  );
}
