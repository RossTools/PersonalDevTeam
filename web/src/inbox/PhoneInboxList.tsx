// The Inbox's list on a phone (PDT-117, design §6): Needs you | Updates, and Review all in the thumb's reach.
import { useState } from 'react';
import { Button, SegmentedControl } from '@ui';
import { cn } from '@ui/lib/utils';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import { NeedsYouGroups, UpdatesList } from './InboxList';
import { step } from './model';

type Segment = 'needs' | 'updates';
// The segment shown last, so coming back from an item (or an update) shows the same one.
let lastSegment: Segment = 'needs';

const heading = 'px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';
/** The bottom bar's buttons: 48px, whatever the pointer. */
const big = 'h-12 pointer-coarse:h-12 min-w-0 flex-1';

/**
 * `/inbox` on a phone: **Needs you** (its count) | **Updates** (a dot while any is unread). A row opens its item;
 * **Review all (n)** at the bottom opens the first and steps through them all. Updates can be cleared one by one or all.
 */
export function PhoneInboxList({ items, updates, loading, onOpen, onReadAll, onClear, onClearAll, onSettings }: {
  items: InboxItem[];
  updates: Notification[];
  loading?: boolean;
  /** Opens an item, or an Update by its selection id. */
  onOpen: (id: string) => void;
  onReadAll: () => void;
  onClear: (id: string) => void;
  onClearAll: () => void;
  onSettings: () => void;
}) {
  const [segment, setSegment] = useState<Segment>(lastSegment);
  const unread = updates.some((u) => u.readAt === null && u.clearedAt === null);
  const first = step(items, null, 1);
  const choose = (s: Segment) => {
    lastSegment = s;
    setSegment(s);
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="px-2 pt-2">
        <SegmentedControl<Segment>
          aria-label="Inbox"
          value={segment}
          onChange={choose}
          className="w-full *:flex-1"
          options={[
            { value: 'needs', label: <>Needs you <span className="tabular-nums">{items.length}</span></>, 'aria-label': `Needs you (${items.length})` },
            {
              value: 'updates',
              label: <>Updates{unread && <span aria-hidden className="size-1.5 rounded-full bg-primary" />}</>,
              'aria-label': unread ? 'Updates (unread)' : 'Updates',
            },
          ]}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-1">
        {segment === 'needs' ? (
          <nav aria-label="Inbox items" className="flex flex-col gap-0.5 py-1">
            <NeedsYouGroups items={items} selectedId={null} onSelect={onOpen} loading={loading} />
          </nav>
        ) : (
          <section aria-label="Updates" className="flex flex-col gap-0.5 py-1">
            <div className="flex items-center gap-1 pr-2">
              <h2 className={cn(heading, 'flex-1')}>Updates</h2>
              <Button size="sm" variant="ghost" disabled={!unread} onClick={onReadAll}>Mark all read</Button>
              <Button size="sm" variant="ghost" disabled={updates.length === 0} onClick={onClearAll}>Clear all</Button>
              <Button size="sm" variant="ghost" onClick={onSettings}>Settings</Button>
            </div>
            <UpdatesList updates={updates} selectedId={null} onSelect={onOpen} onClear={onClear} loading={loading} />
          </section>
        )}
      </div>
      {segment === 'needs' && first && (
        <div className="border-t border-border bg-card px-2 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
          <Button variant="primary" className={cn(big, 'w-full')} onClick={() => onOpen(first)}>Review all ({items.length})</Button>
        </div>
      )}
    </div>
  );
}
