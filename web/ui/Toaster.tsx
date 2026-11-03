import type { ReactNode } from 'react';
import { toast as sonner } from 'sonner';
import { Toaster as ToasterPrimitive } from './primitives/sonner';

/** Render once (in AppShell). Toasts float above everything (`--pdt-z-toast`) and never move the layout. */
export function Toaster() {
  return <ToasterPrimitive position="bottom-right" closeButton />;
}

export interface ToastOptions {
  /** One button on the toast ("Retry", "Open settings"). */
  action?: { label: string; onClick: () => void };
  description?: ReactNode;
  /** Milliseconds; errors stay longer by default. */
  duration?: number;
}

function opts(o: ToastOptions | undefined, defaultDuration?: number) {
  return {
    description: o?.description,
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
