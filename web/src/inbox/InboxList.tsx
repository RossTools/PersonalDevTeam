import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { Badge, Button, EmptyState, IconButton, List, ListHeading, ListRow, RelativeTime } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import { TicketKey } from '../common/TicketKey';
import { grouped, KIND_LABEL, selectedUpdate, updateSelection } from './model';
import { KindIcon } from './kinds';

/** Rows a Needs-you section shows before its Show all. */
const GROUP_ROWS = 50;

/** What a row says under its title: who asks (or which workflow) and the ticket's title. */
function context(i: InboxItem): string {
  const who = i.personaName ?? (typeof i.body.workflow === 'string' ? i.body.workflow : null);
  return [who, i.ticketTitle ?? (i.ticketKey ? null : KIND_LABEL[i.kind])].filter(Boolean).join(' · ');
}

/** An empty list's line: what it says when empty, or that `what` can't be loaded. */
function EmptyLine({ failed, what, children }: { failed?: boolean; what: string; children: string }) {
  return failed
    ? <p role="alert" className="px-3 py-2 text-xs text-failed">Can't load {what}.</p>
    : <EmptyState size="inline" className="px-3 py-2">{children}</EmptyState>;
}

/**
 * Needs you: the items under their sections, most urgent first. Each row: the kind's icon in its colour, the title,
 * then the ticket's key and who asks; the age on the right.
 */
export function NeedsYouGroups({ items, selectedId, onSelect, loading, failed }: {
  items: InboxItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  loading?: boolean;
  /** The Inbox can't be loaded (or the host reached): an empty list says so rather than that nothing waits. */
  failed?: boolean;
}) {
  const groups = grouped(items);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  // Show all goes away when pressed: focus moves to the first row it showed (PDT-547). The list renders its new rows a
  // pass after this one, so it looks for a few frames.
  const reveal = useRef<{ section: Element; id: string } | null>(null);
  useEffect(() => {
    const r = reveal.current;
    if (!r) return;
    reveal.current = null;
    let frame = 0;
    let tries = 10;
    const look = () => {
      const row = [...r.section.querySelectorAll<HTMLElement>('[role="row"][data-key]')].find((el) => el.dataset.key === r.id);
      if (row) row.focus();
      else if (--tries > 0) frame = requestAnimationFrame(look);
    };
    look();
    return () => cancelAnimationFrame(frame);
  }, [expanded]);
  return (
    <>
      {groups.length === 0 && (failed || !loading) && <EmptyLine failed={failed} what="the Inbox">Nothing waits on you.</EmptyLine>}
      {groups.map((g) => {
        // A long section shows its first rows (and the selected one's, wherever it is) until Show all.
        const all = expanded.has(g.id) || g.items.slice(GROUP_ROWS).some((i) => i.id === selectedId);
        const rows = all ? g.items : g.items.slice(0, GROUP_ROWS);
        return (
          <section key={g.id} aria-label={g.title}>
            <ListHeading as="h3">
              {g.title} <Badge count={g.items.length} size="sm" />
            </ListHeading>
            <List aria-label={g.title} onAction={onSelect}>
              {rows.map((i) => (
                <ListRow
                  key={i.id}
                  id={i.id}
                  title={i.title}
                  textValue={i.title}
                  current={i.id === selectedId}
                  className="items-start py-1.5"
                  leading={<KindIcon item={i} className="mt-0.5" />}
                  trailing={<RelativeTime short time={i.createdAt} className="text-meta text-muted-foreground" />}
                  secondary={
                    <span className="flex min-w-0 items-center gap-1.5">
                      {i.ticketKey && <TicketKey ticketKey={i.ticketKey} />}
                      <span className="truncate">{context(i)}</span>
                    </span>
                  }
                />
              ))}
            </List>
            {rows.length < g.items.length && (
              <Button
                variant="ghost"
                size="sm"
                className="mx-2 mt-1"
                onClick={(e) => {
                  const section = e.target.closest('section');
                  if (section) reveal.current = { section, id: g.items[rows.length].id };
                  setExpanded((s) => new Set(s).add(g.id));
                }}
              >
                Show all {g.items.length}
              </Button>
            )}
          </section>
        );
      })}
    </>
  );
}

/**
 * Updates: notifications that wait on nothing, newest first: unread in bold, resolved ones dimmed. With `onClear`, each
 * row has a Clear button (on hover, always on touch).
 */
export function UpdatesList({ updates, selectedId, onSelect, onClear, loading, failed }: {
  updates: Notification[];
  selectedId: string | null;
  onSelect: (selection: string) => void;
  onClear?: (id: string) => void;
  loading?: boolean;
  /** The Updates can't be loaded (or the host reached): an empty list says so rather than that there are none. */
  failed?: boolean;
}) {
  const update = selectedUpdate(selectedId);
  return (
    <>
      {updates.length === 0 && (failed || !loading) && <EmptyLine failed={failed} what="updates">No updates.</EmptyLine>}
      {updates.length > 0 && (
        <List aria-label="Updates" onAction={(id) => onSelect(updateSelection(id))}>
          {updates.map((u) => (
            <ListRow
              key={u.id}
              id={u.id}
              title={u.title}
              textValue={u.title}
              current={u.id === update}
              bold={u.readAt === null && u.clearedAt === null}
              className={u.clearedAt !== null ? 'text-muted-foreground' : undefined}
              secondary={<>{u.body && `${u.body} · `}<RelativeTime time={u.createdAt} /></>}
              trailing={onClear && (
                <IconButton label="Clear" size="sm" onClick={() => onClear(u.id)}
                  className="text-muted-foreground opacity-0 group-data-hovered/row:opacity-100 group-data-focus-visible/row:opacity-100 data-focus-visible:opacity-100 pointer-coarse:opacity-100">
                  <X size={12} aria-hidden />
                </IconButton>
              )}
            />
          ))}
        </List>
      )}
    </>
  );
}

/**
 * The Inbox's list column (PDT-108): **Needs you**, then **Updates**, with Mark all read, Clear all
 * (and Clear on each Update) and a way to the notification settings.
 */
export function InboxList({ items, updates = [], selectedId, onSelect, onReadAll, onClear, onClearAll, onSettings, loading, failed, updatesFailed }: {
  items: InboxItem[];
  updates?: Notification[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onReadAll?: () => void;
  onClear?: (id: string) => void;
  onClearAll?: () => void;
  onSettings?: () => void;
  loading?: boolean;
  failed?: boolean;
  updatesFailed?: boolean;
}) {
  const unread = updates.some((u) => u.readAt === null && u.clearedAt === null);
  return (
    <nav aria-label="Inbox items" className="flex flex-col gap-0.5 py-1">
      <ListHeading>Needs you</ListHeading>
      <NeedsYouGroups items={items} selectedId={selectedId} onSelect={onSelect} loading={loading} failed={failed} />
      <section aria-label="Updates">
        <div className="flex items-center gap-1 pt-1 pr-2">
          <ListHeading className="flex-1">Updates</ListHeading>
          {onReadAll && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" disabled={!unread} onClick={onReadAll}>Mark all read</Button>}
          {onClearAll && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" disabled={updates.length === 0} onClick={onClearAll}>Clear all</Button>}
          {onSettings && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" onClick={onSettings}>Settings</Button>}
        </div>
        <UpdatesList updates={updates} selectedId={selectedId} onSelect={onSelect} onClear={onClear} loading={loading} failed={updatesFailed} />
      </section>
    </nav>
  );
}
