// Snooze (PDT-641): any item can be put out of sight until tomorrow morning, for a week, or until a ticket is done.
// It comes back by itself; Snoozed lists the ones away, with Unsnooze. A snoozed item opens read-only (PDT-686).
import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { AlarmClock, AlarmClockOff } from 'lucide-react';
import { Button, List, ListHeading, ListRow, MenuButton, clockTime, toast } from '@ui';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import type { InboxSnooze } from '../api/gen/InboxSnooze';
import type { SnoozeRequest } from '../api/gen/SnoozeRequest';
import { TicketKey } from '../common/TicketKey';
import { actionError } from '../common/useAction';
import { KindIcon } from './kinds';
import { RowTitle } from './rows';
import { useInbox, useRefreshInbox } from './useInbox';

const WEEK = 7 * 24 * 60 * 60 * 1000;
/** Tomorrow's snooze ends at this local hour. */
const MORNING = 9;

/** 9:00 tomorrow, local time. */
export function tomorrowMorning(now: number = Date.now()): number {
  const d = new Date(now);
  d.setDate(d.getDate() + 1);
  d.setHours(MORNING, 0, 0, 0);
  return d.getTime();
}

/** Until when it is away, in words: "until Tue 9:00 AM", "until PDT-639 is done", or both. */
function snoozeText(s: InboxSnooze, now: number = Date.now()): string {
  const parts = [
    s.ticketId ? `${s.ticketKey ?? 'its ticket'} is done` : null,
    s.until !== null ? clockTime(s.until, { weekday: true, now }) : null,
  ].filter(Boolean);
  return `until ${parts.join(' or ')}`;
}

/**
 * The item's Snooze menu: Until tomorrow, For 1 week, Until a ticket is done… (its own ticket filled in, unless that
 * ticket waits on the item: it would never be done, PDT-698).
 */
export function SnoozeMenu({ item }: { item: InboxItem }) {
  const refresh = useRefreshInbox();
  const snooze = useMutation({
    mutationFn: (s: SnoozeRequest) => api.snoozeInbox(item.id, s),
    // The item leaves the screen: say where it went (PDT-667).
    onSuccess: (_, s) => void toast.success(
      s.ticket ? `Snoozed until ${s.ticket} is done` : s.until != null ? `Snoozed until ${clockTime(s.until, { weekday: true })}` : 'Snoozed',
      { description: item.title },
    ),
    onError: (err) => toast.error(actionError(err)),
    onSettled: () => void refresh(),
  });
  return (
    <MenuButton
      label="Snooze"
      placement="bottom end"
      entries={() => [
        { label: 'Until tomorrow', detail: clockTime(tomorrowMorning(), { weekday: true }), onSelect: () => snooze.mutate({ until: tomorrowMorning() }) },
        { label: 'For 1 week', onSelect: () => snooze.mutate({ until: Date.now() + WEEK }) },
        {
          input: 'Until a ticket is done…',
          defaultValue: item.ticketWaits ? '' : (item.ticketKey ?? ''),
          onSubmit: (v) => { if (v.trim()) snooze.mutate({ ticket: v.trim() }); },
        },
      ]}
    >
      <Button variant="ghost" size="sm" pending={snooze.isPending}><AlarmClock aria-hidden />Snooze</Button>
    </MenuButton>
  );
}

/** Brings a snoozed item back now. */
function useUnsnooze() {
  const refresh = useRefreshInbox();
  return useMutation({ mutationFn: (id: string) => api.unsnoozeInbox(id), onSettled: () => void refresh() });
}

/** Unsnooze in a snoozed item's header, where Snooze sits on the others. */
export function UnsnoozeButton({ item }: { item: InboxItem }) {
  const unsnooze = useUnsnooze();
  return (
    <Button variant="ghost" size="sm" pending={unsnooze.isPending} onClick={() => unsnooze.mutate(item.id)}>
      <AlarmClockOff aria-hidden />Unsnooze
    </Button>
  );
}

/** Until when a snoozed item is away, or "snoozed". */
export function snoozedLine(i: InboxItem): string {
  return i.snooze ? snoozeText(i.snooze) : 'snoozed';
}

/** Rows Snoozed shows before Show all. */
const SHOWN = 3;

/**
 * The Inbox list's Snoozed section: what is away and until when, with Unsnooze. A row opens its item (read-only) with
 * `onSelect`.
 */
export function Snoozed({ selectedId = null, onSelect }: { selectedId?: string | null; onSelect?: (id: string) => void }) {
  const items = useInbox().data?.snoozed ?? [];
  const unsnooze = useUnsnooze();
  const [all, setAll] = useState(false);
  if (items.length === 0) return null;
  const shown = all ? items : items.slice(0, SHOWN);
  return (
    <section aria-label="Snoozed">
      <ListHeading as="h3">Snoozed</ListHeading>
      <List aria-label="Snoozed" onAction={onSelect}>
        {shown.map((i) => (
          <ListRow
            key={i.id}
            id={i.id}
            title={<RowTitle>{i.title}</RowTitle>}
            textValue={i.title}
            current={i.id === selectedId}
            className="items-start py-1.5"
            leading={<KindIcon item={i} className="mt-0.5" />}
            secondary={
              <span className="flex min-w-0 items-center gap-1.5">
                {i.ticketKey && <TicketKey ticketKey={i.ticketKey} />}
                <span className="truncate">
                  {snoozedLine(i)}
                  {i.ticketWaits && ` · ${i.ticketKey ?? 'its ticket'} waits on it`}
                </span>
              </span>
            }
            trailing={
              <Button size="xs" variant="ghost" pending={unsnooze.isPending && unsnooze.variables === i.id}
                onClick={() => unsnooze.mutate(i.id)}>Unsnooze</Button>
            }
          />
        ))}
      </List>
      {!all && shown.length < items.length && (
        <Button variant="ghost" size="sm" className="mx-2 mt-1" onClick={() => setAll(true)}>Show all {items.length}</Button>
      )}
    </section>
  );
}
