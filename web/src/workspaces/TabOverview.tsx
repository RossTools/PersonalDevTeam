// The phone's tab overview (PDT-114): every tab of the workspace as a card with a live snippet. ⧉ in the tab strip
// opens it; Back closes it.
import { useQuery } from '@tanstack/react-query';
import { FileDiff, Ticket } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { ChoiceCard, ContextMenu, DiffStat } from '@ui';
import { api } from '../api/client';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Tab } from '../api/gen/Tab';
import { AgentDot, AgentLabel, itemState } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { useInbox } from '../inbox/useInbox';
import { useDiffSize } from '../review/queries';
import { runLabel } from '../runs/runState';
import { useBackCloses } from '../shell/router';
import { tabMenu } from './menus';
import { itemOf } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';

/** How many of a conversation's newest events are read for its card. */
const LAST_EVENTS = 20;

/** The last non-empty line of some Markdown, without its list, heading or quote marks. */
export function lastLine(markdown: string): string | null {
  const lines = markdown.split('\n').map((l) => l.replace(/^\s*(?:[-*+>]|#{1,6}|\d+\.)\s+/, '').replace(/[*_`]/g, '').trim());
  return lines.filter(Boolean).pop() ?? null;
}

/**
 * Pure: what a session's card says. An open question (the Inbox's, else the conversation's own request) comes first,
 * then the agent's last line.
 */
export function sessionSnippet(events: ConversationEvent[], inbox: InboxItem[]): { text: string; question: boolean } | null {
  const asked = inbox.find((i) => i.kind === 'question' || i.kind === 'approval');
  if (asked) return { text: asked.title, question: true };
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i].event;
    if (e.kind === 'request' && e.state === 'open') return { text: e.requestKind === 'approval' ? `Wants ${e.tool}` : e.title, question: true };
    if (e.kind === 'agent_message') {
      const line = lastLine(e.markdown);
      if (line) return { text: line, question: false };
    }
  }
  return null;
}

function SessionSnippet({ info, inbox }: { info: ItemInfo; inbox: InboxItem[] }) {
  const s = info.session!;
  const conversation = s.mode === 'conversation';
  const events = useQuery({
    queryKey: ['session-last-events', s.id],
    queryFn: () => api.lastSessionEvents(s.id, LAST_EVENTS),
    enabled: conversation,
    refetchInterval: 5000,
  });
  const snippet = sessionSnippet(events.data ?? [], inbox.filter((i) => i.sessionId === s.id));
  if (!snippet) return null;
  return <p className={cn('line-clamp-2 text-meta', snippet.question ? 'text-foreground' : 'text-muted-foreground')}>{snippet.question ? `? ${snippet.text}` : snippet.text}</p>;
}

function Size({ worktreeId }: { worktreeId: string }) {
  const size = useDiffSize(worktreeId);
  return size && size.files > 0 ? <DiffStat added={size.added} removed={size.removed} className="text-meta" /> : null;
}

function Card({ ws, tab, info, current, inbox, onOpen, onRename }: {
  ws: WorkspacesApi;
  tab: Tab;
  info: ItemInfo | null;
  current: boolean;
  inbox: InboxItem[];
  onOpen: () => void;
  onRename: () => void;
}) {
  const title = info?.title ?? (tab.sessionId ? 'Session' : tab.ticketId ? 'Ticket' : tab.worktreeId ? 'Changes' : 'Run');
  const state = info && (info.session || info.run) ? itemState(info) : null;
  return (
    <ContextMenu label={`Tab ${title}`} entries={() => tabMenu(ws, tab, onRename)}>
      <ChoiceCard selection="current" selected={current} size="sm" onClick={onOpen} className="min-h-24 justify-start bg-card py-2.5">
        <span className="flex min-w-0 items-center gap-1.5 text-sm">
          {tab.ticketId ? <Ticket aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            : tab.worktreeId ? <FileDiff aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            : state && <AgentDot state={state} />}
          <SessionKindIcon session={info?.session} />
          <span className={cn('truncate font-medium', info?.unseen && 'font-semibold')}>{title}</span>
        </span>
        <span className="flex min-w-0 flex-wrap items-center gap-x-2 text-meta text-muted-foreground">
          {state?.label && <AgentLabel state={state} />}
          {info?.run && !state?.label && <span>{runLabel(info.run.state)}</span>}
          {info?.ticket && <span>{STAGE_LABEL[info.ticket.stage]}</span>}
          {info?.worktreeId && (tab.worktreeId || info.session) && <Size worktreeId={info.worktreeId} />}
        </span>
        {info?.session && <SessionSnippet info={info} inbox={inbox} />}
      </ChoiceCard>
    </ContextMenu>
  );
}

/**
 * The tabs of the workspace on screen as a grid of cards: each with its state, a live snippet (the agent's last line
 * or open question, the branch's `+added −removed`). A tap opens the tab; a long press has the tab's menu (Rename,
 * Move, Close). Back closes it.
 */
export function TabOverview({ ws, onClose, onRename }: {
  ws: WorkspacesApi;
  onClose: () => void;
  /** Renames a tab in the tab strip (the overview closes first). */
  onRename: (tabId: string) => void;
}) {
  useBackCloses(true, onClose);
  const inbox = useInbox().data?.items ?? [];
  const w = ws.workspace;
  return (
    <section aria-label="Tab overview" className="flex-1 overflow-auto p-2">
      {w && w.tabs.length > 0 ? (
        <div className="grid grid-cols-2 gap-2">
          {w.tabs.map((t) => (
            <Card
              key={t.id}
              ws={ws}
              tab={t}
              info={ws.info(itemOf(t))}
              current={t.id === ws.tab?.id}
              inbox={inbox}
              onOpen={() => { ws.selectTab(t); onClose(); }}
              onRename={() => { ws.selectTab(t); onClose(); onRename(t.id); }}
            />
          ))}
        </div>
      ) : (
        <p className="p-4 text-center text-sm text-muted-foreground">No tabs here.</p>
      )}
    </section>
  );
}
