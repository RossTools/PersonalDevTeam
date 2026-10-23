import { lazy, Suspense, useEffect, useState } from 'react';
import { BriefcaseBusiness } from 'lucide-react';
import { Button, EmptyState, List, ListRow, PromptBox, Skeleton, ViewHeader, relativeTime } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { useAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { ConversationView } from '../conversation/ConversationView';
import { enterAddsLine } from '../conversation/ConversationComposer';
import { loadDraft, saveDraft } from '../conversation/drafts';
import { earlier, firstLine, noteOf, REASON_LABEL, usePm, useRefreshPm, type PmSelection } from './usePm';

/** The start box's unsent text, kept like a conversation's (it has no session yet). */
export const PM_START_DRAFT = 'pm-start';

const NotesPage = lazy(() => import('./NotesPage').then((m) => ({ default: m.NotesPage })));
const NotePage = lazy(() => import('./NotesPage').then((m) => ({ default: m.NotePage })));

/**
 * The PM view's work area (PDT-81): the selected PM session's conversation (an ended one read-only, with its
 * journal), the notes or one note, the list of earlier sessions, or, with no current session, a box whose first
 * message starts one. On a phone (PDT-113) the conversation is the whole screen: the top bar has the title and ⋯, and
 * a tap on a note opens it as a screen of its own.
 */
export function PmView({ selection, onSelect, onNew, onBack }: {
  selection: PmSelection;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
  /** Leaves a screen that is gone (a deleted note): Back. */
  onBack?: () => void;
}) {
  const pm = usePm();
  const phone = useMediaQuery(PHONE_QUERY);
  const note = noteOf(selection);
  if (selection === 'notes' || note) {
    return (
      <Suspense fallback={<Skeleton lines={8} className="p-4" />}>
        {note
          ? <NotePage key={note} id={note} onGone={onBack ?? (() => onSelect('notes'))} />
          : <NotesPage onOpen={phone ? (id) => onSelect(`note:${id}`) : undefined} />}
      </Suspense>
    );
  }
  if (pm.isPending) return <Skeleton lines={8} className="p-4" />;
  if (selection === 'earlier') return <EarlierSessions onSelect={onSelect} phone={phone} />;
  const sessions = pm.data?.sessions ?? [];
  const current = pm.data?.current ?? null;
  const info = selection === 'current'
    ? sessions.find((s) => s.session.id === current?.id)
    : sessions.find((s) => s.session.id === selection);
  if (!info) return <PmStart phone={phone} onStarted={() => onSelect('current')} />;
  const s = info.session;
  const ended = info.rotation === 'done';
  const meta = [
    `Started ${relativeTime(s.createdAt)}`,
    ended ? `ended: ${info.reason ? REASON_LABEL[info.reason] : 'ended'}` : null,
    s.persona ? `${s.persona.name} v${s.persona.version}` : null,
  ].filter(Boolean).join(' · ');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {phone ? (
        ended && <p className="border-b border-border px-4 py-1.5 text-meta text-muted-foreground">{meta}</p>
      ) : (
        <ViewHeader
          title={ended ? 'Earlier PM session' : 'Project manager'}
          leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />}
          meta={<span className="text-meta">{meta}</span>}
          actions={<Button variant="ghost" size="sm" onClick={onNew}>New session</Button>}
        />
      )}
      <ConversationView
        key={s.id}
        session={s}
        className="min-h-0 flex-1"
        banner={info.rotation === 'journal' ? (
          <p role="status" className="px-4 pb-1 text-center text-meta text-muted-foreground">
            This session is writing its journal; your next message starts a new one.
          </p>
        ) : undefined}
        readOnly={ended ? <Ended journal={info.journal} onCurrent={() => onSelect('current')} /> : undefined}
      />
    </div>
  );
}

function Ended({ journal, onCurrent }: { journal: string | null; onCurrent: () => void }) {
  return (
    <div className="flex flex-col gap-2 border-t border-border px-4 py-3 text-sm">
      {journal && (
        <p className="line-clamp-4 whitespace-pre-wrap text-muted-foreground"><span className="font-medium text-foreground">Journal: </span>{journal}</p>
      )}
      <div className="flex items-center gap-2">
        <span className="flex-1 text-muted-foreground">This session has ended.</span>
        <Button size="sm" onClick={onCurrent}>Go to the current session</Button>
      </div>
    </div>
  );
}

/** The sessions that ended, newest first: when, how it ended, and its journal's first line; each opens read-only. */
function EarlierSessions({ onSelect, phone }: { onSelect: (s: PmSelection) => void; phone: boolean }) {
  const pm = usePm();
  const ended = earlier(pm.data?.sessions ?? []);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!phone && <ViewHeader title="Earlier PM sessions" leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />} />}
      <div className="min-h-0 flex-1 overflow-auto px-1 py-1">
        {ended.length === 0 ? (
          <EmptyState className="mt-16 border-0">No PM session has ended yet.</EmptyState>
        ) : (
          <List aria-label="Earlier PM sessions" onAction={(id) => onSelect(String(id))}>
            {ended.map((s) => (
              <ListRow
                key={s.session.id}
                id={s.session.id}
                title={new Date(s.session.createdAt).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}
                secondary={
                  <span className="flex flex-col">
                    <span>Ended: {s.reason ? REASON_LABEL[s.reason] : 'ended'}</span>
                    {firstLine(s.journal) && <span className="truncate">{firstLine(s.journal)}</span>}
                  </span>
                }
              />
            ))}
          </List>
        )}
      </div>
    </div>
  );
}

/**
 * No current session: the first message starts one (its briefing is made then). Unsent text is kept. A phone doesn't
 * focus the box (the keyboard would cover the text) and follows the composer's Enter rule.
 */
function PmStart({ onStarted, phone }: { onStarted: (s: Session) => void; phone: boolean }) {
  const [text, setText] = useState(() => loadDraft(PM_START_DRAFT));
  useEffect(() => saveDraft(PM_START_DRAFT, text), [text]);
  const refresh = useRefreshPm();
  const [start, starting] = useAction(async () => {
    const s = await api.startPm(text.trim());
    setText('');
    await refresh();
    onStarted(s);
  });
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!phone && <ViewHeader title="Project manager" leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />} />}
      <EmptyState icon={<BriefcaseBusiness />} className="mt-16 border-0">
        Ask about any project: what's in flight, why something is stuck, or have it draft a ticket or a workflow. It
        starts from a briefing of PDT's state and its notes, and asks before it changes anything.
      </EmptyState>
      <div className="mt-auto px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <PromptBox
          layout={phone ? 'phone' : 'default'}
          label="Ask the PM"
          placeholder="Ask the PM…"
          value={text}
          onChange={setText}
          pending={starting}
          canSend={!!text.trim() && !starting}
          onSend={() => void start()}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing && !enterAddsLine(phone) && text.trim()) {
              e.preventDefault();
              void start();
            }
          }}
          autoFocus={!phone}
        />
      </div>
    </div>
  );
}
