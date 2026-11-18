// Recently answered (PDT-623): the Inbox's last answers, what each did and how it was given, with Undo on the ones
// still waiting out their window. "What did I just press?" has an answer here.
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, List, ListHeading, ListRow, RelativeTime } from '@ui';
import { api } from '../api/client';
import type { InboxAnswered } from '../api/gen/InboxAnswered';
import { isRecord } from '../common/guards';
import { useOpenTicket } from '../common/open';
import { POLL } from '../common/poll';
import { TicketKey } from '../common/TicketKey';
import { undoAnswer, useHeldAnswers } from './held';
import { inboxKey } from './useInbox';

/** How many answers the list keeps, and shows before Show all. */
const KEPT = 10;
const SHOWN = 3;

/** Under the Inbox's key, so answering refreshes it with the list. */
const answeredKey = [...inboxKey, 'answered'] as const;

/** A choice as the list says it: `allow_always` → "Always allow", `outcome:approved` → "Approved". */
function choiceText(choice: string | null): string {
  if (!choice) return 'Answered';
  if (choice === 'allow_always') return 'Always allow';
  const words = choice.replace(/^outcome:/, '').replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What it did and how: "Request changes · keyboard · phone", or by an agent. */
function answeredLine(a: InboxAnswered): string {
  const what = [choiceText(a.choice), a.answer ? `“${a.answer.length > 60 ? `${a.answer.slice(0, 59)}…` : a.answer}”` : null]
    .filter(Boolean).join(': ');
  const how = a.by !== 'user' ? ['by an agent'] : a.source ? [a.source.via, a.source.device] : [];
  return [what, ...how].join(' · ');
}

function answeredOf(payload: unknown): InboxAnswered | null {
  return isRecord(payload) && typeof payload.itemId === 'string' && typeof payload.title === 'string' ? payload as InboxAnswered : null;
}

/** The section: answers held for Undo first (with Undo), then the last ones given, newest first. */
export function RecentlyAnswered() {
  const held = useHeldAnswers();
  const events = useQuery({
    queryKey: answeredKey,
    queryFn: () => api.listLogEvents({ type: 'inbox.answered', limit: KEPT }),
    refetchInterval: POLL.live,
  });
  const [all, setAll] = useState(false);
  const openTicket = useOpenTicket();
  const answered = (events.data ?? []).flatMap((e) => {
    const a = answeredOf(e.payload);
    return a && !held.some((h) => h.itemId === a.itemId) ? [{ id: e.id, at: e.createdAt, a }] : [];
  });
  if (held.length === 0 && answered.length === 0) return null;
  const shown = all ? answered : answered.slice(0, Math.max(0, SHOWN - held.length));
  const open = (id: string) => {
    const a = answered.find((x) => x.id === id)?.a;
    if (a?.ticketId && a.projectId) openTicket({ id: a.ticketId, projectId: a.projectId });
  };
  return (
    <section aria-label="Recently answered">
      <ListHeading as="h3">Recently answered</ListHeading>
      <List aria-label="Recently answered" onAction={open}>
        {[
          ...held.map((h) => (
            <ListRow
              key={h.itemId}
              id={`held:${h.itemId}`}
              title={h.title}
              textValue={h.title}
              className="py-1.5"
              secondary={`${h.label} · ${h.sending ? 'sending' : 'waiting to send'}`}
              trailing={!h.sending && (
                <Button size="sm" variant="ghost" className="h-5 px-1.5 text-2xs" onClick={() => undoAnswer(h.itemId)}>Undo</Button>
              )}
            />
          )),
          ...shown.map(({ id, at, a }) => (
            <ListRow
              key={id}
              id={id}
              title={a.title}
              textValue={a.title}
              className="items-start py-1.5"
              trailing={<RelativeTime short time={at} className="text-meta text-muted-foreground" />}
              secondary={
                <span className="flex min-w-0 items-center gap-1.5">
                  {a.ticketKey && <TicketKey ticketKey={a.ticketKey} />}
                  <span className="truncate">{answeredLine(a)}</span>
                </span>
              }
            />
          )),
        ]}
      </List>
      {!all && shown.length < answered.length && (
        <Button variant="ghost" size="sm" className="mx-2 mt-1" onClick={() => setAll(true)}>Show all {answered.length}</Button>
      )}
    </section>
  );
}
