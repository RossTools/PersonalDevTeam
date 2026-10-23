import { lazy, Suspense, useEffect, useState } from 'react';
import { BriefcaseBusiness } from 'lucide-react';
import { Button, EmptyState, PromptBox, Skeleton, ViewHeader, relativeTime } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { useAction } from '../common/useAction';
import { ConversationView } from '../conversation/ConversationView';
import { loadDraft, saveDraft } from '../conversation/drafts';
import { REASON_LABEL, usePm, useRefreshPm, type PmSelection } from './usePm';

/** The start box's unsent text, kept like a conversation's (it has no session yet). */
export const PM_START_DRAFT = 'pm-start';

const NotesPage = lazy(() => import('./NotesPage').then((m) => ({ default: m.NotesPage })));

/**
 * The PM view's work area (PDT-81): the selected PM session's conversation (an ended one read-only, with its
 * journal), the notes, or, with no current session, a box whose first message starts one.
 */
export function PmView({ selection, onSelect, onNew }: {
  selection: PmSelection;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
}) {
  const pm = usePm();
  if (selection === 'notes') {
    return <Suspense fallback={<Skeleton lines={8} className="p-4" />}><NotesPage /></Suspense>;
  }
  if (pm.isPending) return <Skeleton lines={8} className="p-4" />;
  const sessions = pm.data?.sessions ?? [];
  const current = pm.data?.current ?? null;
  const info = selection === 'current'
    ? sessions.find((s) => s.session.id === current?.id)
    : sessions.find((s) => s.session.id === selection);
  if (!info) return <PmStart onStarted={() => onSelect('current')} />;
  const s = info.session;
  const ended = info.rotation === 'done';
  const meta = [
    `Started ${relativeTime(s.createdAt)}`,
    ended ? `ended: ${info.reason ? REASON_LABEL[info.reason] : 'ended'}` : null,
    s.persona ? `${s.persona.name} v${s.persona.version}` : null,
  ].filter(Boolean).join(' · ');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={ended ? 'Earlier PM session' : 'Project manager'}
        leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />}
        meta={<span className="text-meta">{meta}</span>}
        actions={<Button variant="ghost" size="sm" onClick={onNew}>New session</Button>}
      />
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

/** No current session: the first message starts one (its briefing is made then). Unsent text is kept. */
function PmStart({ onStarted }: { onStarted: (s: Session) => void }) {
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
      <ViewHeader title="Project manager" leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />} />
      <EmptyState icon={<BriefcaseBusiness />} className="mt-16 border-0">
        Ask about any project: what's in flight, why something is stuck, or have it draft a ticket or a workflow. It
        starts from a briefing of PDT's state and its notes, and asks before it changes anything.
      </EmptyState>
      <div className="mt-auto p-3">
        <PromptBox
          label="Ask the PM"
          placeholder="Ask the PM…"
          value={text}
          onChange={setText}
          pending={starting}
          canSend={!!text.trim() && !starting}
          onSend={() => void start()}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && text.trim()) {
              e.preventDefault();
              void start();
            }
          }}
          autoFocus
        />
      </div>
    </div>
  );
}
