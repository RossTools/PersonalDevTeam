// The ticket tab's Conversation section: the ticket's agent sessions, newest
// first, in a switcher above the chosen one's conversation. The session keeps its own tab too.
import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Play } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, EmptyState, useConfirm } from '@ui';
import { api } from '../api/client';
import type { AttemptSession } from '../api/gen/AttemptSession';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import { AgentDot, agentState, itemState } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { TicketKey } from '../common/TicketKey';
import { runAction, useAction } from '../common/useAction';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { isAcp } from '../common/agents';
import { ConversationView } from '../conversation/ConversationView';
import type { EventsSocketLike } from '../conversation/eventsSocket';
import { invalidateSessions } from '../sessions/queries';
import { abandonAttempt, canStart, startNextPart, useOpenSession, useOpenTicket, useStartTicket } from './actions';
import { isClosed } from './model';
import { invalidateTickets, useAllSessions } from './queries';

function stateOf(s: AttemptSession, live: Session | undefined) {
  return live ? itemState({ session: live, unseen: live.unseen }) : agentState({ kind: 'agent_tui', status: s.status });
}

/**
 * No session on the ticket yet: what to do depends on where it is. A ticket that can start offers Start; one at work
 * with no agent (a branch made by hand, or its agent ended) offers an agent on its branch, or Abandon…; a parent points
 * at its parts, which have the conversations; a closed ticket has nothing to offer.
 */
function NoConversation({ ticket: t }: { ticket: Ticket }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const start = useStartTicket();
  const openTicket = useOpenTicket();
  const defaults = useAgentDefaults();
  const active = t.attempt?.state === 'active' ? t.attempt : null;
  const [startAgent, starting] = useAction(async () => {
    if (!active) return;
    const agent = defaults.agent;
    await api.startAttemptAgent(active.id, {
      agent,
      sessionMode: isAcp(agent) ? 'conversation' : defaults.mode,
      permissionMode: defaults.ticketPermissionMode,
    });
    invalidateTickets(qc, [t.id]);
    void invalidateSessions(qc, t.projectId);
  });
  const [abandon, abandoning] = useAction(() => abandonAttempt(confirm, qc, t, active!));
  if (t.partList.length > 0) {
    return (
      <EmptyState
        action={
          <>
            <ul aria-label="Parts" className="flex max-w-full flex-col items-center gap-1 text-sm">
              {t.partList.map((p) => (
                <li key={p.id} className="flex min-w-0 max-w-full items-center gap-1.5">
                  <TicketKey ticketKey={p.key} title={`Open ${p.key}`} onClick={() => openTicket({ id: p.id, projectId: t.projectId })} />
                  <span className="min-w-0 truncate">{p.title}</span>
                </li>
              ))}
            </ul>
            {!isClosed(t.stage) && t.partList.some((p) => !isClosed(p.stage)) && (
              <Button size="sm" onClick={() => void runAction(() => startNextPart(start, t))}><Play aria-hidden /> Start next part</Button>
            )}
          </>
        }
      >
        Each part has its own conversation.
      </EmptyState>
    );
  }
  if (active) {
    return (
      <EmptyState
        action={
          <div className="flex flex-wrap justify-center gap-2">
            <Button variant="primary" size="sm" pending={starting} onClick={() => void startAgent()}><Play aria-hidden /> Start an agent here</Button>
            <Button variant="ghost" size="sm" pending={abandoning} onClick={() => void abandon()}>Abandon…</Button>
          </div>
        }
      >
        No agent is working on <span className="font-mono">{active.branch}</span>.
      </EmptyState>
    );
  }
  if (canStart(t)) {
    return (
      <EmptyState action={<Button variant="primary" size="sm" onClick={() => start(t, { here: true })}><Play aria-hidden /> Start</Button>}>
        No agent has worked on this yet.
      </EmptyState>
    );
  }
  return <EmptyState>No agent worked on this ticket.</EmptyState>;
}

export function TicketConversation({ ticket: t, socketFactory }: {
  ticket: Ticket;
  /** Tests: a fake events WebSocket. */
  socketFactory?: (url: string) => EventsSocketLike;
}) {
  const all = useAllSessions();
  const openSession = useOpenSession();
  const [picked, setPicked] = useState<string | null>(null);
  const sessions = t.sessions;
  const chosen = sessions.find((s) => s.id === picked) ?? sessions.find((s) => s.mode === 'conversation') ?? sessions[0];

  if (!chosen) return <NoConversation ticket={t} />;
  const live = all.find((x) => x.id === chosen.id);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {sessions.length > 1 && (
        <div role="group" aria-label="Sessions" className="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-border px-3 py-1.5">
          {sessions.map((s) => {
            const l = all.find((x) => x.id === s.id);
            const state = stateOf(s, l);
            return (
              <button
                key={s.id}
                type="button"
                aria-pressed={s.id === chosen.id}
                onClick={() => setPicked(s.id)}
                className={cn(
                  'flex max-w-64 shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-xs outline-none hover:bg-muted',
                  'focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch',
                  s.id === chosen.id ? 'bg-accent font-medium text-foreground' : 'text-muted-foreground',
                )}
              >
                <AgentDot state={state} />
                <SessionKindIcon session={l ?? { kind: 'agent_tui', agent: s.agent }} />
                <span className="min-w-0 truncate">{l?.title ?? s.title}</span>
                {state.label && <span className="shrink-0 text-meta text-muted-foreground">{state.label}</span>}
              </button>
            );
          })}
        </div>
      )}
      {chosen.mode !== 'conversation' ? (
        <EmptyState action={<Button size="sm" onClick={() => openSession({ id: chosen.id, projectId: t.projectId })}>Open its tab</Button>}>
          {chosen.title} runs in the agent's terminal.
        </EmptyState>
      ) : live ? (
        <ConversationView key={live.id} session={live} socketFactory={socketFactory} className="min-h-0 flex-1" />
      ) : (
        <EmptyState>Loading the conversation…</EmptyState>
      )}
    </div>
  );
}
