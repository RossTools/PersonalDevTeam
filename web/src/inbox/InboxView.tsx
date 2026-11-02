import { useEffect, useRef } from 'react';
import { Button, EmptyState, ViewHeader, relativeTime } from '@ui';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { useOpenTicket } from '../tickets/actions';
import { after, selectedUpdate, step } from './model';
import { useClearUpdate, useReadNotification } from '../notifications/queries';
import { useAnswer } from './useAnswer';
import { useItemView } from './itemView';
import { ItemLayout } from './ItemLayout';

/**
 * Keys typed into a field, or with a dialog or menu open, are not the Inbox's; nor is Enter on a focused button or
 * link, which presses that (Reject, Dismiss, …), not the primary decision.
 */
function foreign(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (e.ctrlKey || e.metaKey || e.altKey) return true;
  if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return true;
  if (e.key === 'Enter' && t?.closest?.('button, a[href], [role=button], [role=link], [role=radio], [role=checkbox], [role=switch], [role=option], [role=menuitem], [role=tab]')) return true;
  return !!document.querySelector('[role=dialog], [role=menu], [role=listbox]');
}

/**
 * The Inbox's work area (UX spec §8.4): the selected item in the one item layout (PDT-361). Triage keys: `j`/`k`
 * move, Enter takes the primary action, `o` opens the ticket, `e` dismisses where that makes sense, number keys pick
 * a question's choices, `y`/`n` answer an approval; answering moves to the next item. An Update (PDT-108) shows what
 * it says and opens what it is about.
 */
export function InboxView({ items, updates = [], selectedId, onSelect, onOpenTarget, failed }: {
  items: InboxItem[];
  /** The Inbox can't be loaded (or the host reached): no item says so rather than that nothing waits. */
  failed?: boolean;
  updates?: Notification[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenTarget?: (t: NotificationTarget) => void;
}) {
  const updateId = selectedUpdate(selectedId);
  const update = updateId ? updates.find((u) => u.id === updateId) ?? null : null;
  const item = updateId ? null : items.find((i) => i.id === selectedId) ?? null;

  // An item that went away (answered elsewhere): show the first one left.
  useEffect(() => {
    if (!item && !updateId && items.length > 0) onSelect(step(items, null, 1));
  }, [item, updateId, items, onSelect]);
  // Opening an item reads its notifications.
  const itemId = item && !item.id.startsWith('session:') ? item.id : null;
  useEffect(() => {
    if (itemId) void api.readNotifications({ inboxItemId: itemId }).catch(() => {});
  }, [itemId]);

  if (updateId) {
    return update ? <UpdateView update={update} onOpenTarget={onOpenTarget} onCleared={() => onSelect(null)} /> : <EmptyState>This update is gone.</EmptyState>;
  }
  if (!item) {
    if (items.length === 0 && failed) return <EmptyState>Can't load the Inbox. It shows again once the host answers.</EmptyState>;
    return <EmptyState>{items.length === 0 ? 'Nothing waits on you. Questions, approvals and gates from agents and workflows show up here.' : 'Choose an item.'}</EmptyState>;
  }
  return <ItemPane key={item.id} item={item} items={items} onSelect={onSelect} />;
}

/** One item, keyed by it: its note, answers and Take over are its own. */
function ItemPane({ item, items, onSelect }: { item: InboxItem; items: InboxItem[]; onSelect: (id: string | null) => void }) {
  const openTicket = useOpenTicket();
  const { busy, answer, dismiss, refresh } = useAnswer(item, (done) => onSelect(after(items, done.id)));
  const { spec, note, setNote } = useItemView({
    item, busy, answer, dismiss: () => void dismiss(),
    onApproved: () => { onSelect(after(items, item.id)); void refresh(); },
  });

  // The handlers change every render; the listener reads the latest through a ref.
  const keys = useRef({ spec, dismiss, item, items, onSelect, openTicket });
  keys.current = { spec, dismiss, item, items, onSelect, openTicket };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (foreign(e)) return;
      const k = keys.current;
      switch (e.key) {
        case 'j': k.onSelect(step(k.items, k.item.id, 1)); break;
        case 'k': k.onSelect(step(k.items, k.item.id, -1)); break;
        case 'Enter': {
          const p = k.spec.enter ? k.spec.actions.find((a) => a.primary) : undefined;
          if (p && !p.disabled && !p.pending) p.run();
          break;
        }
        case 'o':
          if (k.item.ticketId && k.item.projectId) k.openTicket({ id: k.item.ticketId, projectId: k.item.projectId });
          break;
        case 'e': void k.dismiss(); break;
        default:
          if (!k.spec.onKey?.(e.key)) return;
      }
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return <ItemLayout item={item} spec={spec} note={note} onNote={setNote} />;
}

/** An Update: what it says, read once it is open, a way to what it is about, and Clear (then `onCleared`). */
export function UpdateView({ update, onOpenTarget, onCleared }: {
  update: Notification;
  onOpenTarget?: (t: NotificationTarget) => void;
  onCleared?: () => void;
}) {
  const read = useReadNotification();
  const clear = useClearUpdate();
  const unread = update.readAt === null;
  const { mutate } = read;
  useEffect(() => {
    if (unread) mutate(update.id);
  }, [unread, update.id, mutate]);
  const open = update.target.kind !== 'none' && onOpenTarget ? () => onOpenTarget(update.target) : null;
  const meta = [update.severity === 'error' ? 'Error' : update.severity === 'warning' ? 'Warning' : null,
    relativeTime(update.createdAt), update.clearedAt !== null ? 'resolved' : null].filter(Boolean).join(' · ');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={update.title}
        titleTooltip={update.title}
        meta={<span className="text-meta text-muted-foreground">{meta}</span>}
        actions={<>
          {open && <Button variant="ghost" size="sm" onClick={open}>Open</Button>}
          <Button variant="ghost" size="sm" onClick={() => { clear.mutate(update.id); onCleared?.(); }}>Clear</Button>
        </>}
      />
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        {update.body && <p className="text-sm whitespace-pre-wrap">{update.body}</p>}
        {open && <div><Button variant="primary" onClick={open}>Open</Button></div>}
      </div>
    </div>
  );
}

