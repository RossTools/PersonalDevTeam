import { useEffect, useRef, type ReactNode } from 'react';
import { cn } from './lib/utils';

export interface Suggestion {
  id: string;
  label: string;
  /** Muted text after the label (a command's description, a file's folder). */
  detail?: string;
  /** Shown but not pickable; `hint` says why ("Only in the terminal"). */
  disabled?: boolean;
  hint?: string;
}

export interface SuggestionListProps {
  /** The listbox id, for the text field's `aria-controls`; options are `${id}-${index}`. */
  id: string;
  /** Accessible name ("Commands", "Files"). */
  label: string;
  items: Suggestion[];
  /** The highlighted index (keyboard), or -1. */
  active: number;
  onPick: (index: number) => void;
  onHover?: (index: number) => void;
  /** Shown when there are no items (e.g. "No matching files"). */
  empty?: ReactNode;
  className?: string;
}

/** The option id for `aria-activedescendant`. */
export const suggestionId = (listId: string, index: number) => `${listId}-${index}`;

/**
 * Completions for what is being typed in a text field (`/` commands, `@` files): a listbox the field drives with the
 * keyboard (it keeps focus; arrows, Enter/Tab and Escape stay with the field). Presentational: the field owns
 * `active` and picking.
 */
export function SuggestionList({ id, label, items, active, onPick, onHover, empty, className }: SuggestionListProps) {
  const list = useRef<HTMLUListElement>(null);
  // Keep the highlighted option in view as the arrows move it.
  useEffect(() => {
    const el = list.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);
  return (
    <div className={cn('overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-popover', className)}>
      {items.length === 0 ? (
        <p className="px-3 py-2 text-xs text-muted-foreground">{empty ?? 'No matches'}</p>
      ) : (
        <ul ref={list} id={id} role="listbox" aria-label={label} className="max-h-64 overflow-y-auto py-1">
          {items.map((it, i) => (
            <li
              key={it.id}
              id={suggestionId(id, i)}
              data-index={i}
              role="option"
              aria-selected={i === active}
              aria-disabled={it.disabled || undefined}
              // The text field keeps focus: picking with the mouse must not blur it.
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => onHover?.(i)}
              onClick={() => !it.disabled && onPick(i)}
              className={cn(
                'flex min-h-control-md cursor-default items-baseline gap-2 px-3 py-1 text-sm pointer-coarse:min-h-control-touch',
                i === active && 'bg-accent',
                it.disabled && 'opacity-50',
              )}
            >
              <span className="shrink-0 font-mono text-xs">{it.label}</span>
              {it.detail && <span className="min-w-0 truncate text-xs text-muted-foreground">{it.detail}</span>}
              {it.hint && <span className="ml-auto shrink-0 text-2xs text-muted-foreground">{it.hint}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
