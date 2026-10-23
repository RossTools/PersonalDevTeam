import { Button, List, ListRow, relativeTime } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import { TicketKey } from '../common/TicketKey';
import { cn } from '@ui/lib/utils';
import { grouped, KIND_LABEL, selectedUpdate, updateSelection } from './model';

const heading = 'px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/**
 * The Inbox's list column (UX spec §8.4, PDT-108 §5.1): **Needs you**, items grouped by kind, most urgent first (the
 * ticket's key, the item's title, its kind, persona and age); then **Updates**, notifications that wait on nothing,
 * newest first: unread in bold, cleared ones dimmed.
 */
export function InboxList({ items, updates = [], selectedId, onSelect, onReadAll, onSettings, loading }: {
  items: InboxItem[];
  updates?: Notification[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onReadAll?: () => void;
  onSettings?: () => void;
  loading?: boolean;
}) {
  const groups = grouped(items);
  const update = selectedUpdate(selectedId);
  const unread = updates.some((u) => u.readAt === null && u.clearedAt === null);
  return (
    <nav aria-label="Inbox items" className="flex flex-col gap-0.5 py-1">
      <h2 className={heading}>Needs you</h2>
      {groups.length === 0 && !loading && <p className="px-3 py-2 text-xs text-muted-foreground">Nothing waits on you.</p>}
      {groups.map((g) => (
        <section key={g.id} aria-label={g.title}>
          <h3 className={heading}>{g.title} <span className="tabular-nums">{g.items.length}</span></h3>
          <List aria-label={g.title} onAction={onSelect}>
            {g.items.map((i) => (
              <ListRow
                key={i.id}
                id={i.id}
                title={i.title}
                textValue={i.title}
                current={i.id === selectedId}
                leading={i.ticketKey ? <TicketKey ticketKey={i.ticketKey} /> : undefined}
                secondary={[KIND_LABEL[i.kind], i.personaName, relativeTime(i.createdAt)].filter(Boolean).join(' · ')}
              />
            ))}
          </List>
        </section>
      ))}
      <section aria-label="Updates">
        <div className="flex items-center gap-1 pt-1 pr-2">
          <h2 className={cn(heading, 'flex-1')}>Updates</h2>
          {onReadAll && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" disabled={!unread} onClick={onReadAll}>Mark all read</Button>}
          {onSettings && <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" onClick={onSettings}>Settings</Button>}
        </div>
        {updates.length === 0 && !loading && <p className="px-3 py-2 text-xs text-muted-foreground">No updates.</p>}
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
                className={u.clearedAt !== null ? 'opacity-60' : undefined}
                secondary={[u.body, relativeTime(u.createdAt)].filter(Boolean).join(' · ')}
              />
            ))}
          </List>
        )}
      </section>
    </nav>
  );
}
