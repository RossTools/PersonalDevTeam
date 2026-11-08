import type { ReactNode } from 'react';
import { toast as sonner } from 'sonner';
import { Toaster as ToasterPrimitive } from './primitives/sonner';

/**
 * Render once (in AppShell). Toasts float above everything (`--pdt-z-toast`) and never move the layout. On a desktop
 * they sit high enough to clear the bars fixed at the bottom of a view (an Inbox item's action bar, a conversation's
 * composer), so a toast never covers Deny or Send; phones keep Sonner's own offset.
 */
export function Toaster() {
  return <ToasterPrimitive position="bottom-right" closeButton offset={{ bottom: '8rem', right: '1.5rem' }} />;
}

export interface ToastOptions {
  /** One button on the toast ("Retry", "Open settings"). */
  action?: { label: string; onClick: () => void };
  description?: ReactNode;
  /** Replaces the kind's icon (a question's "?"). */
  icon?: ReactNode;
  /** Milliseconds; errors stay longer by default. */
  duration?: number;
}

function opts(o: ToastOptions | undefined, defaultDuration?: number) {
  return {
    description: o?.description,
    icon: o?.icon,
    duration: o?.duration ?? defaultDuration,
    action: o?.action ? { label: o.action.label, onClick: () => o.action!.onClick() } : undefined,
  };
}

/** Results and errors of background actions: `toast.error(e.message, { action: { label, onClick } })`. */
export const toast = {
  success: (message: ReactNode, o?: ToastOptions) => sonner.success(message, opts(o)),
  error: (message: ReactNode, o?: ToastOptions) => sonner.error(message, opts(o, 8000)),
  info: (message: ReactNode, o?: ToastOptions) => sonner.info(message, opts(o)),
  dismiss: (id?: string | number) => sonner.dismiss(id),
};
