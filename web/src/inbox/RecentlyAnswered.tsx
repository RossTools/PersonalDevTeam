// Recently answered (PDT-623): the Inbox's last answers, what each did and how it was given, with Undo on the ones
// still waiting out their window. "What did I just press?" has an answer here.
import { useQuery } from '@tanstack/react-query';
import { Button, ListRow, RelativeTime } from '@ui';
import { capitalize } from '../common/text';
import { api } from '../api/client';
import type { InboxAnswered } from '../api/gen/InboxAnswered';
import { useKeepFocus } from '../common/focusLost';
import { isRecord } from '../common/guards';
import { useOpenTicket } from '../common/open';
import { POLL } from '../common/poll';
import { TicketKey } from '../common/TicketKey';
import { type HeldAnswer, undoAnswer, useHeldAnswers } from './held';
import { CollapsibleListSection } from './ListSection';
import { keepFocusOnReturn } from './snooze';
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
  return capitalize(words);
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

type Row = { held: HeldAnswer } | { id: string; at: number; a: InboxAnswered };

/** The section: answers held for Undo first (with Undo), then the last ones given, newest first. */
export function RecentlyAnswered() {
  const held = useHeldAnswers();
  const events = useQuery({
    queryKey: answeredKey,
    queryFn: () => api.listLogEvents({ type: 'inbox.answered', limit: KEPT }),
    refetchInterval: POLL.live,
  });
  const openTicket = useOpenTicket();
  // Undo takes its row away (the section, with the last): focus goes on to the next row, or the item's row back in
  // Needs you (PDT-668).
  const keep = useKeepFocus();
  const answered = (events.data ?? []).flatMap((e) => {
    const a = answeredOf(e.payload);
    return a && !held.some((h) => h.itemId === a.itemId) ? [{ id: e.id, at: e.createdAt, a }] : [];
  });
  if (held.length === 0 && answered.length === 0) return null;
  const rows: Row[] = [...held.map((h) => ({ held: h })), ...answered];
  const open = (id: string) => {
    const a = answered.find((x) => x.id === id)?.a;
    if (a?.ticketId && a.projectId) openTicket({ id: a.ticketId, projectId: a.projectId });
  };
  return (
    <CollapsibleListSection
      label="Recently answered"
      items={rows}
      rowId={(r) => ('held' in r ? `held:${r.held.itemId}` : r.id)}
      // Every held answer shows, with Undo.
      limit={Math.max(SHOWN, held.length)}
      onAction={open}
    >
      {(r) => 'held' in r ? (
        <ListRow
          key={r.held.itemId}
          id={`held:${r.held.itemId}`}
          title={r.held.title}
          textValue={r.held.title}
          className="py-1.5"
          secondary={`${r.held.label} · ${r.held.sending ? 'sending' : 'waiting to send'}`}
          trailing={!r.held.sending && (
            <Button size="xs" variant="ghost" onClick={(e) => {
              keepFocusOnReturn(keep, e.target, r.held.itemId);
              undoAnswer(r.held.itemId);
            }}>Undo</Button>
          )}
        />
      ) : (
        <ListRow
          key={r.id}
          id={r.id}
          title={r.a.title}
          textValue={r.a.title}
          className="items-start py-1.5"
          trailing={<RelativeTime short time={r.at} className="text-meta text-muted-foreground" />}
          secondary={
            <span className="flex min-w-0 items-center gap-1.5">
              {r.a.ticketKey && <TicketKey ticketKey={r.a.ticketKey} />}
              <span className="truncate">{answeredLine(r.a)}</span>
            </span>
          }
        />
      )}
    </CollapsibleListSection>
  );
}
