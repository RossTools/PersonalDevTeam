import { List, ListRow, relativeTime } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import { TicketKey } from '../common/TicketKey';
import { grouped, KIND_LABEL } from './model';

const heading = 'px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/**
 * The Inbox's list column (UX spec §8.4): items grouped by kind, most urgent first. Each row: the ticket's key, the
 * item's title, and its kind, persona and age.
 */
export function InboxList({ items, selectedId, onSelect, loading }: {
  items: InboxItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  loading?: boolean;
}) {
  const groups = grouped(items);
  return (
    <nav aria-label="Inbox items" className="flex flex-col gap-0.5 py-1">
      <h2 className={heading}>Inbox</h2>
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
    </nav>
  );
}
