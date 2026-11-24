// One Inbox list section (PDT-668): its heading, its first rows, and Show all, which moves focus to the first row it
// shows as it goes away (PDT-547). Needs you's sections, Snoozed and Recently answered all use it.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, List, ListHeading } from '@ui';
import { listRows } from '../common/focusLost';

export function CollapsibleListSection<T>({ label, heading, items, rowId, limit, open = false, onAction, children }: {
  label: string;
  /** What the heading says (a count badge, say); `label` without one. */
  heading?: ReactNode;
  items: T[];
  /** The row's id in the List, as its `ListRow` has it. */
  rowId: (item: T) => string;
  /** Rows shown before Show all. */
  limit: number;
  /** Shows every row without Show all (the selected one is past the limit). */
  open?: boolean;
  onAction?: (id: string) => void;
  /** A row: a `ListRow` with `id={rowId(item)}`. */
  children: (item: T) => ReactNode;
}) {
  const [all, setAll] = useState(false);
  const section = useRef<HTMLElement>(null);
  // The id of the first row Show all reveals, until it has focus. The List renders its new rows a pass after this one,
  // so it looks for a few frames.
  const reveal = useRef<string | null>(null);
  useEffect(() => {
    const id = reveal.current;
    if (!id) return;
    reveal.current = null;
    let frame = 0;
    let tries = 10;
    const look = () => {
      const row = listRows(section.current).find((el) => el.dataset.key === id);
      if (row) row.focus();
      else if (--tries > 0) frame = requestAnimationFrame(look);
    };
    look();
    return () => cancelAnimationFrame(frame);
  }, [all]);
  const shown = all || open ? items : items.slice(0, limit);
  return (
    <section ref={section} aria-label={label}>
      <ListHeading as="h3">{heading ?? label}</ListHeading>
      <List aria-label={label} onAction={onAction}>
        {shown.map(children)}
      </List>
      {shown.length < items.length && (
        <Button
          variant="ghost"
          size="sm"
          className="mx-2 mt-1"
          onClick={() => {
            reveal.current = rowId(items[shown.length]);
            setAll(true);
          }}
        >
          Show all {items.length}
        </Button>
      )}
    </section>
  );
}

/**
 * For `useKeepFocus`: the row that takes a removed row's place in its List (the one before, when it was the last),
 * else what `fallback` finds (the whole section went).
 */
export function rowInPlaceOf(from: Element, fallback?: () => HTMLElement | null | undefined): () => HTMLElement | null | undefined {
  const list = from.closest('[role="grid"]');
  const at = Math.max(0, listRows(list).findIndex((r) => r.contains(from)));
  return () => {
    const rows = list?.isConnected ? listRows(list) : [];
    return rows[Math.min(at, rows.length - 1)] ?? fallback?.();
  };
}
