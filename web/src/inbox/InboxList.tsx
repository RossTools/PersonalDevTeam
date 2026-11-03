import { X } from 'lucide-react';
import { Button, IconButton, List, ListRow, RelativeTime } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import { TicketKey } from '../common/TicketKey';
import { cn } from '@ui/lib/utils';
import { grouped, KIND_LABEL, selectedUpdate, updateSelection } from './model';
import { KindIcon } from './kinds';

const heading = 'px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/** What a row says under its title: who asks (or which workflow) and the ticket's title. */
function context(i: InboxItem): string {
  const who = i.personaName ?? (typeof i.body.workflow === 'string' ? i.body.workflow : null);
  return [who, i.ticketTitle ?? (i.ticketKey ? null : KIND_LABEL[i.kind])].filter(Boolean).join(' · ');
}

/** An empty list's line: what it says when empty, or that `what` can't be loaded. */
function EmptyLine({ failed, what, children }: { failed?: boolean; what: string; children: string }) {
  return failed
    ? <p role="alert" className="px-3 py-2 text-xs text-failed">Can't load {what}.</p>
    : <p className="px-3 py-2 text-xs text-muted-foreground">{children}</p>;
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
  return (
    <>
      {groups.length === 0 && (failed || !loading) && <EmptyLine failed={failed} what="the Inbox">Nothing waits on you.</EmptyLine>}
      {groups.map((g) => (
        <section key={g.id} aria-label={g.title}>
          <h3 className={cn(heading, 'flex items-center gap-1.5')}>
            {g.title} <span className="rounded-full bg-surface-2 px-1.5 tabular-nums">{g.items.length}</span>
          </h3>
          <List aria-label={g.title} onAction={onSelect}>
            {g.items.map((i) => (
              <ListRow
                key={i.id}
                id={i.id}
                title={i.title}
                textValue={i.title}
                current={i.id === selectedId}
                className="items-start py-1.5 data-current:shadow-[inset_2px_0_0_var(--pdt-accent)]"
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
        </section>
      ))}
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
      <h2 className={heading}>Needs you</h2>
      <NeedsYouGroups items={items} selectedId={selectedId} onSelect={onSelect} loading={loading} failed={failed} />
      <section aria-label="Updates">
        <div className="flex items-center gap-1 pt-1 pr-2">
          <h2 className={cn(heading, 'flex-1')}>Updates</h2>
          {onReadAll && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" disabled={!unread} onClick={onReadAll}>Mark all read</Button>}
          {onClearAll && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" disabled={updates.length === 0} onClick={onClearAll}>Clear all</Button>}
          {onSettings && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" onClick={onSettings}>Settings</Button>}
        </div>
        <UpdatesList updates={updates} selectedId={selectedId} onSelect={onSelect} onClear={onClear} loading={loading} failed={updatesFailed} />
      </section>
    </nav>
  );
}
