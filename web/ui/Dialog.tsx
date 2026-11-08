import { useId, type ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { Button } from './Button';
import {
  Dialog as DialogPrimitive,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from './primitives/dialog';

export type DialogSize = 'sm' | 'md' | 'lg' | 'xl' | 'full' | 'gallery';

const sizes: Record<DialogSize, string> = {
  sm: 'sm:max-w-sm',
  md: 'sm:max-w-md',
  lg: 'sm:max-w-2xl',
  /** Wide content: a diff. */
  xl: 'sm:max-w-6xl',
  /** Most of the screen, at its full height: a long text being edited. */
  full: 'h-[calc(100dvh-4rem)] sm:max-w-[calc(100vw-4rem)]',
  /** A large picker of fixed height whose body lays out its own scrolling columns (the New workflow gallery). */
  gallery: 'sm:max-w-5xl h-[min(42.5rem,calc(100dvh-2rem))]',
};

/**
 * A modal: title, optional description, body, footer actions. Escape, the close button and the backdrop close it;
 * focus is trapped inside and returns to what opened it. Controlled with `open`/`onOpenChange`; pass `trigger` (a
 * Button) to have it open the dialog by itself (then `open` is optional). A dialog that is mounted only while it shows
 * (`{editing && <Dialog …/>}`) passes `onClose` instead: it is open, and closing it calls `onClose`.
 */
export function Dialog({
  open,
  onOpenChange,
  onClose,
  trigger,
  title,
  description,
  children,
  footer,
  headerActions,
  size = 'md',
  dismissable = true,
  className,
}: {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Called when it closes (Escape, the close button, the backdrop); without `open`, it is open. */
  onClose?: () => void;
  trigger?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  /** Action buttons, right-aligned (primary last). */
  footer?: ReactNode;
  /** Beside the title, before the close button (a search field). */
  headerActions?: ReactNode;
  size?: DialogSize;
  /** false: the backdrop doesn't close it (Escape still does). For forms with unsaved text. */
  dismissable?: boolean;
  className?: string;
}) {
  const descId = useId();
  const change = onClose
    ? (o: boolean) => {
      onOpenChange?.(o);
      if (!o) onClose();
    }
    : onOpenChange;
  const modal = (
    <DialogPrimitive
      {...(trigger ? {} : { isOpen: open ?? onClose !== undefined })}
      onOpenChange={trigger ? undefined : change}
      isDismissable={dismissable}
      dialogProps={description ? { 'aria-describedby': descId } : undefined}
      // Header, scrolling body, footer: the rows live on the inner dialog element (the modal box's only child), so the
      // box is a column that caps the height and the inner grid fills it.
      className={cn('flex max-h-[calc(100dvh-2rem)] flex-col', sizes[size], className)}
      innerClassName="grid min-h-0 flex-1 grid-rows-[auto_minmax(0,1fr)_auto] gap-3"
    >
      <DialogHeader className={cn('pr-8', headerActions && 'flex-row items-center gap-3')}>
        <DialogTitle className="shrink-0 text-sm font-semibold">{title}</DialogTitle>
        {description && (
          <DialogDescription id={descId} className="text-xs">
            {description}
          </DialogDescription>
        )}
        {headerActions && <div className="flex min-w-0 flex-1 items-center gap-2">{headerActions}</div>}
      </DialogHeader>
      <div className="min-h-0 overflow-auto">{children}</div>
      {/* A footer of key hints alone (KeyHints) goes with them on touch screens. */}
      {footer && <DialogFooter className="pointer-coarse:has-[>[data-slot=key-hints]:only-child]:hidden">{footer}</DialogFooter>}
    </DialogPrimitive>
  );
  if (!trigger) return modal;
  return (
    <DialogTrigger {...(open === undefined ? {} : { isOpen: open })} onOpenChange={change}>
      {trigger}
      {modal}
    </DialogTrigger>
  );
}

/**
 * A dialog's usual footer: Cancel (secondary), then the action that names what it does ("Create", "Merge"), primary
 * or `danger`. `extra` goes first, on the left of Cancel. `form` makes the action submit that form (by id) instead of
 * calling `onAction`.
 */
export function DialogActions({
  onCancel,
  cancel = 'Cancel',
  action,
  onAction,
  pending,
  disabled,
  danger,
  form,
  extra,
}: {
  onCancel: () => void;
  cancel?: ReactNode;
  action: ReactNode;
  onAction?: () => void;
  pending?: boolean;
  disabled?: boolean;
  danger?: boolean;
  form?: string;
  extra?: ReactNode;
}) {
  return (
    <>
      {extra}
      <Button variant="secondary" onClick={onCancel}>{cancel}</Button>
      <Button
        variant={danger ? 'danger' : 'primary'}
        pending={pending}
        disabled={disabled}
        {...(form ? { type: 'submit' as const, form } : { onClick: onAction })}
      >
        {action}
      </Button>
    </>
  );
}
