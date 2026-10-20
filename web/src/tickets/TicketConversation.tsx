// The ticket tab's Conversation section (structured agents spec §10, UX spec §8.2): the ticket's agent sessions, newest
// first, in a switcher above the chosen one's conversation. The session keeps its own tab too.
import { useState } from 'react';
import { Play } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, EmptyState } from '@ui';
import type { AttemptSession } from '../api/gen/AttemptSession';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import { AgentDot, agentState, itemState } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { ConversationView } from '../conversation/ConversationView';
import type { EventsSocketLike } from '../conversation/eventsSocket';
import { canStart, useOpenSession, useStartTicket } from './actions';
import { useAllSessions } from './queries';

function stateOf(s: AttemptSession, live: Session | undefined) {
  return live ? itemState({ session: live, unseen: live.unseen }) : agentState({ kind: 'agent_tui', status: s.status });
}

export function TicketConversation({ ticket: t, socketFactory }: {
  ticket: Ticket;
  /** Tests: a fake events WebSocket. */
  socketFactory?: (url: string) => EventsSocketLike;
}) {
  const all = useAllSessions();
  const openSession = useOpenSession();
  const start = useStartTicket();
  const [picked, setPicked] = useState<string | null>(null);
  const sessions = t.sessions;
  const chosen = sessions.find((s) => s.id === picked) ?? sessions.find((s) => s.mode === 'conversation') ?? sessions[0];

  if (!chosen) {
    return (
      <EmptyState
        action={canStart(t) && (
          <Button variant="primary" size="sm" onClick={() => start(t, { here: true })}><Play aria-hidden /> Start</Button>
        )}
      >
        No conversation yet. Start the ticket to give it to an agent.
      </EmptyState>
    );
  }
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
