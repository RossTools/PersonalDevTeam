import type { ReactNode } from 'react';
import { X } from 'lucide-react';
import { cn } from './lib/utils';
import { Button } from './Button';

export interface BulkActionBarProps {
  /** How many items are selected (the bar is meant to show only when this is > 0). */
  count: number;
  /** The noun after the count: "3 selected" by default, or e.g. "3 tickets selected". */
  noun?: string;
  /** Action buttons and menus; pass `size="sm" compact` so the bar keeps its height on touch screens. */
  children?: ReactNode;
  /** Clears the selection (the Clear button and Escape inside the bar). */
  onClear: () => void;
  className?: string;
}

/**
 * Replaces a list's `FilterBar` while rows are selected: "N selected", the actions, Clear. Exactly h-10, like the
 * filter row on desktop, so swapping one for the other never moves the list.
 */
export function BulkActionBar({ count, noun, children, onClear, className }: BulkActionBarProps) {
  return (
    <div
      role="toolbar"
      aria-label="Bulk actions"
      data-slot="bulk-action-bar"
      onKeyDown={(e) => {
        if (e.key !== 'Escape' || e.defaultPrevented) return;
        e.preventDefault();
        onClear();
      }}
      className={cn('flex h-10 shrink-0 items-center gap-2 bg-accent px-3 text-sm', className)}
    >
      <span aria-live="polite" className="shrink-0 font-medium tabular-nums">
        {count} {noun ? `${noun} selected` : 'selected'}
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto py-1">{children}</div>
      <Button variant="ghost" size="sm" compact onClick={onClear} className="shrink-0">
        <X aria-hidden /> Clear
      </Button>
    </div>
  );
}
