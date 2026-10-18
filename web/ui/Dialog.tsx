import { useId, type ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import {
  Dialog as DialogPrimitive,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from './primitives/dialog';

export type DialogSize = 'sm' | 'md' | 'lg';

const sizes: Record<DialogSize, string> = {
  sm: 'sm:max-w-sm',
  md: 'sm:max-w-md',
  lg: 'sm:max-w-2xl',
};

/**
 * A modal: title, optional description, body, footer actions. Escape, the close button and the backdrop close it;
 * focus is trapped inside and returns to what opened it. Controlled with `open`/`onOpenChange`; pass `trigger` (a
 * Button) to have it open the dialog by itself (then `open` is optional).
 */
export function Dialog({
  open,
  onOpenChange,
  trigger,
  title,
  description,
  children,
  footer,
  size = 'md',
  dismissable = true,
  className,
}: {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  trigger?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  /** Action buttons, right-aligned (primary last). */
  footer?: ReactNode;
  size?: DialogSize;
  /** false: the backdrop doesn't close it (Escape still does). For forms with unsaved text. */
  dismissable?: boolean;
  className?: string;
}) {
  const descId = useId();
  const modal = (
    <DialogPrimitive
      {...(trigger ? {} : { isOpen: open ?? false })}
      onOpenChange={trigger ? undefined : onOpenChange}
      isDismissable={dismissable}
      dialogProps={description ? { 'aria-describedby': descId } : undefined}
      // Header, scrolling body, footer: the rows live on the inner dialog element (the modal box's only child), so the
      // box is a column that caps the height and the inner grid fills it.
      className={cn('flex max-h-[calc(100dvh-2rem)] flex-col', sizes[size], className)}
      innerClassName="grid min-h-0 flex-1 grid-rows-[auto_minmax(0,1fr)_auto] gap-3"
    >
      <DialogHeader className="pr-8">
        <DialogTitle className="text-sm font-semibold">{title}</DialogTitle>
        {description && (
          <DialogDescription id={descId} className="text-xs">
            {description}
          </DialogDescription>
        )}
      </DialogHeader>
      <div className="min-h-0 overflow-auto">{children}</div>
      {footer && <DialogFooter>{footer}</DialogFooter>}
    </DialogPrimitive>
  );
  if (!trigger) return modal;
  return (
    <DialogTrigger {...(open === undefined ? {} : { isOpen: open })} onOpenChange={onOpenChange}>
      {trigger}
      {modal}
    </DialogTrigger>
  );
}
