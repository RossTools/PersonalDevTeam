import { useEffect, useState } from 'react';
import { BriefcaseBusiness, FileStack } from 'lucide-react';
import { Button, EmptyState, List, ListRow, PromptBox, RelativeTime, Skeleton, ViewHeader, absoluteTime } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { useAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { ConversationView } from '../conversation/ConversationView';
import { DocumentsBesideLayout, useOpenBesidePicker } from '../documents/BesidePanel';
import { enterAddsLine } from '../conversation/ConversationComposer';
import { loadDraft, saveDraft } from '../conversation/drafts';
import { useViewTracking } from '../sessions/useViewTracking';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { RotateReason } from '../api/gen/RotateReason';
import { InlineRename } from '../common/InlineRename';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { earlier, firstLine, REASON_LABEL, resetting, shownConversation, usePm, usePmActions, useRefreshPm, useShownRef, useViaLabel, type PmSelection } from './usePm';

/** The start box's unsent text, kept like a conversation's (it has no session yet). */
export const PM_START_DRAFT = 'pm-start';

/**
 * The PM view's work area (PDT-81, PDT-149): the conversation on show (an ended one read-only, with its journal), the
 * list of ended ones, or the box whose first message starts a new conversation (also shown when none is open). On a
 * phone (PDT-113) the conversation is the whole screen: the top bar has its title, and ⋯ renames, resets or closes it.
 * A reset conversation (PDT-308) links to the one it carries on from, read-only.
 * The notes have a view of their own (PDT-143).
 */
export function PmView({ selection, onSelect, onNew }: {
  selection: PmSelection;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
}) {
  const pm = usePm();
  const phone = useMediaQuery(PHONE_QUERY);
  const shown = selection === 'new' || selection === 'earlier' ? undefined
    : shownConversation(pm.data?.sessions, pm.data?.current?.id, selection);
  const onShow = useShownRef(shown?.session.id ?? null);
  if (pm.isPending) return <Skeleton lines={8} className="p-4" />;
  if (selection === 'earlier') return <EarlierSessions onSelect={onSelect} phone={phone} />;
  const info = shown;
  if (!info) return <PmStart phone={phone} onStarted={(s) => onSelect(s.id)} />;
  const previous = info.resetFrom ? pm.data?.sessions.find((s) => s.session.id === info.resetFrom) : undefined;
  // A reset puts the fresh conversation on screen in the old one's place, if the old one is still on show (PDT-308).
  const onReset = (old: string, fresh: string) => {
    if (onShow.current === old) onSelect(fresh);
  };
  return <Conversation key={info.session.id} info={info} previous={previous} phone={phone} onSelect={onSelect} onNew={onNew} onReset={onReset} />;
}

function Conversation({ info, previous, phone, onSelect, onNew, onReset }: {
  info: PmSessionInfo;
  /** The conversation this one was reset from (PDT-308). */
  previous?: PmSessionInfo;
  onReset: (old: string, fresh: string) => void;
  phone: boolean;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
}) {
  const s = info.session;
  const ended = !info.open;
  const [renaming, setRenaming] = useState(false);
  const viaLabel = useViaLabel();
  const { rename, close, reset } = usePmActions(() => onSelect('current'), onReset);
  const canClose = info.rotation === 'none';
  // Documents beside the conversation (PDT-317).
  const besidePicker = useOpenBesidePicker(s.id);
  useTopBarActions(phone && !ended ? () => [
    { label: 'Rename', onSelect: () => setRenaming(true) },
    besidePicker.entry,
    { label: 'Reset…', onSelect: () => reset(info), disabled: !canClose },
    { label: 'Close…', onSelect: () => close(info), disabled: !canClose },
  ] : null);
  const rest = [
    ended ? `ended: ${info.reason ? REASON_LABEL[info.reason] : 'ended'}` : null,
    s.persona ? `${s.persona.name} v${s.persona.version}` : null,
    viaLabel(info),
  ].filter(Boolean).join(' · ');
  const meta = <>Started <RelativeTime time={s.createdAt} />{rest ? ` · ${rest}` : ''}</>;
  const renameField = (
    <InlineRename
      label="Conversation name"
      initial={info.title}
      onCommit={(t) => { setRenaming(false); rename(s.id, t); }}
      onCancel={() => setRenaming(false)}
    />
  );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {phone ? (
        <>
          {renaming && <div className="border-b border-border px-3 py-1.5">{renameField}</div>}
          {ended && <p className="border-b border-border px-4 py-1.5 text-meta text-muted-foreground">{meta}</p>}
        </>
      ) : (
        <ViewHeader
          title={renaming ? renameField : info.title}
          titleTooltip={info.title}
          leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />}
          meta={meta}
          actions={(
            <>
              <Button variant="ghost" size="sm" onClick={besidePicker.pick}><FileStack aria-hidden /> Open beside…</Button>
              {!ended && <Button variant="ghost" size="sm" onClick={() => setRenaming(true)}>Rename</Button>}
              {!ended && <Button variant="ghost" size="sm" disabled={!canClose} onClick={() => reset(info)}>Reset…</Button>}
              {!ended && <Button variant="ghost" size="sm" disabled={!canClose} onClick={() => close(info)}>Close…</Button>}
              <Button variant="ghost" size="sm" onClick={onNew}>New conversation</Button>
            </>
          )}
        />
      )}
      {previous && (
        <div className="border-b border-border px-2 py-1">
          <Button variant="ghost" size="sm" onClick={() => onSelect(previous.session.id)}>
            Previous conversation (reset {absoluteTime(previous.lastActivity)})
          </Button>
        </div>
      )}
      <ViewTracked session={s} />
      {besidePicker.dialog}
      <DocumentsBesideLayout sessionId={s.id} phone={phone}>
        <ConversationView
          session={s}
          className="min-h-0 flex-1"
          banner={resetting(info) ? (
            <p role="status" className="px-4 pb-1 text-center text-meta text-muted-foreground">
              Resetting: this conversation is writing its journal, then a fresh one starts from it here.
            </p>
          ) : info.rotation === 'journal' ? (
            <p role="status" className="px-4 pb-1 text-center text-meta text-muted-foreground">
              You closed this conversation: it is writing its journal, then it ends. Start a new one to carry on.
            </p>
          ) : undefined}
          readOnly={ended ? <Ended reason={info.reason} journal={info.journal} onNew={onNew} /> : undefined}
        />
      </DocumentsBesideLayout>
    </div>
  );
}

/** Marks the PM session on screen seen, as a session's tab does (it clears the rail's dot, PDT-145). */
function ViewTracked({ session }: { session: Session }) {
  useViewTracking(session.id, session.unseen ? session.attentionAt ?? 0 : null);
  return null;
}

/**
 * The notice under an ended conversation (PDT-167): why it ended (only the user's Close ends one now; older ones may
 * have ended on a full context or at a day old), its journal, and New conversation.
 */
function Ended({ reason, journal, onNew }: { reason: RotateReason | null; journal: string | null; onNew: () => void }) {
  return (
    <div role="status" className="flex flex-col gap-2 border-t border-border px-4 py-3 text-sm">
      {journal && (
        <p className="line-clamp-4 whitespace-pre-wrap text-muted-foreground"><span className="font-medium text-foreground">Journal: </span>{journal}</p>
      )}
      <div className="flex items-center gap-2">
        <span className="flex-1 text-muted-foreground">
          {reason ? `This conversation ended because ${REASON_LABEL[reason]}.` : 'This conversation has ended.'} Its journal
          carries into new conversations.
        </span>
        <Button size="sm" onClick={onNew}>New conversation</Button>
      </div>
    </div>
  );
}

/** The conversations that ended, newest first: when, how it ended, and its journal's first line; each opens read-only. */
function EarlierSessions({ onSelect, phone }: { onSelect: (s: PmSelection) => void; phone: boolean }) {
  const pm = usePm();
  const ended = earlier(pm.data?.sessions ?? []);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!phone && <ViewHeader title="Ended PM conversations" leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />} />}
      <div className="min-h-0 flex-1 overflow-auto px-1 py-1">
        {ended.length === 0 ? (
          <EmptyState className="mt-16 border-0">No PM conversation has ended yet.</EmptyState>
        ) : (
          <List aria-label="Ended PM conversations" onAction={(id) => onSelect(String(id))}>
            {ended.map((s) => (
              <ListRow
                key={s.session.id}
                id={s.session.id}
                title={s.title}
                secondary={
                  <span className="flex flex-col">
                    <span>{absoluteTime(s.session.createdAt)}</span>
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
 * A new conversation, beside the open ones (PDT-149): the first message starts it and names it (its briefing is made then). Unsent text is kept. A phone doesn't
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
      {!phone && <ViewHeader title="New conversation" leading={<BriefcaseBusiness aria-hidden className="size-4 text-muted-foreground" />} />}
      <EmptyState icon={<BriefcaseBusiness />} className="mt-16 border-0">
        Ask about any project: what's in flight, why something is stuck, or have it draft a ticket or a workflow. It
        starts from a briefing of PDT's state, its notes and your other open conversations, and asks before it changes
        anything. Keep one conversation per topic: the others stay open.
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
