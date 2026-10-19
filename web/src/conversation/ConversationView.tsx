import type { ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { MessagesSquare } from 'lucide-react';
import { EmptyState, toast, type ApprovalAnswer } from '@ui';
import { cn } from '@ui/lib/utils';
import { api, ApiError, type SessionPatch } from '../api/client';
import type { Delivery } from '../api/gen/Delivery';
import type { Session } from '../api/gen/Session';
import { actionError, runAction } from '../common/useAction';
import { latestUsage } from './blocks';
import { ConversationComposer } from './ConversationComposer';
import type { EventsSocketLike } from './eventsSocket';
import { MessageList } from './MessageList';
import { useConversation, type ConversationData } from './useConversation';

const thousands = (n: number) => (n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));

/** "Context 45% · 90k / 200k" from the latest usage report. */
function contextReadout(usage: ReturnType<typeof latestUsage>): string | null {
  if (!usage?.contextUsed || !usage.contextWindow) return null;
  const pct = Math.round((usage.contextUsed / usage.contextWindow) * 100);
  return `Context ${pct}% · ${thousands(usage.contextUsed)} / ${thousands(usage.contextWindow)}`;
}

/**
 * An agent session shown as a conversation (spec §7): the message list over the composer. Sending also reopens a
 * parked, stopped or ended conversation, so there is no separate restart flow.
 */
export function ConversationView({ session, socketFactory, className }: {
  session: Session;
  /** Tests: a fake WebSocket. */
  socketFactory?: (url: string) => EventsSocketLike;
  className?: string;
}) {
  const conv = useConversation(session.id, socketFactory);
  return <ConversationPane session={session} conv={conv} className={className} />;
}

/** The conversation for events the caller follows (SessionView shares one socket between its header and this). */
export function ConversationPane({ session, conv, banner, className }: {
  session: Session;
  conv: ConversationData;
  /** A quiet line over the composer (e.g. "Also open in the terminal"). */
  banner?: ReactNode;
  className?: string;
}) {
  const qc = useQueryClient();
  const state = conv.state;
  const turnRunning = state?.turnRunning ?? false;
  const agent = session.agent ?? 'claude';
  const agentName = agent === 'codex' ? 'Codex' : 'Claude';
  const caps = useQuery({ queryKey: ['agent-capabilities', agent], queryFn: () => api.agentCapabilities(agent), staleTime: 60_000 });

  const refreshSessions = () => qc.invalidateQueries({ queryKey: ['sessions', session.projectId] });
  const patch = useMutation({
    mutationFn: (p: SessionPatch) => api.updateSession(session.id, p),
    onSuccess: () => void refreshSessions(),
    onError: (err) => toast.error(actionError(err)),
  });
  const shownModel = patch.isPending && patch.variables.model !== undefined ? patch.variables.model : session.model;
  const shownMode =
    patch.isPending && patch.variables.permissionMode
      ? patch.variables.permissionMode
      : state?.permissionMode ?? session.permissionMode ?? 'ask';

  const send = async (text: string, delivery?: Delivery) => {
    await api.sendMessage(session.id, { text, ...(delivery ? { delivery } : {}) });
    // A parked or ended session comes back to life; the list shows it sooner.
    if (session.status !== 'running') void refreshSessions();
  };
  const answer = async (requestId: string, a: ApprovalAnswer) => {
    try {
      await api.answerRequest(session.id, requestId, a);
    } catch (err) {
      if (err instanceof ApiError && err.code === 'request_closed') toast.info('That request was already answered or has expired.');
      else toast.error(actionError(err));
    }
  };
  const interrupt = () => void runAction(() => api.interruptSession(session.id));

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <MessageList
        rows={conv.rows}
        provisional={conv.provisional}
        streams={conv.streams}
        turnRunning={turnRunning}
        pending={state?.pending ?? []}
        onAnswer={answer}
        empty={
          conv.ready ? (
            <EmptyState icon={<MessagesSquare />} className="mt-16 border-0">
              Ask {agentName} something. It works in {session.cwd}.
            </EmptyState>
          ) : null
        }
      />
      {banner}
      {conv.status === 'reconnecting' && (
        <p className="px-4 pb-1 text-center text-meta text-muted-foreground" role="status">Reconnecting…</p>
      )}
      <ConversationComposer
        turnRunning={turnRunning}
        onSend={send}
        onInterrupt={interrupt}
        stopped={state?.stopped ?? null}
        agentName={agentName}
        models={caps.data?.models ?? []}
        model={shownModel ?? null}
        onModel={(model) => patch.mutate({ model })}
        permissionModes={caps.data?.permissionModes ?? []}
        permissionMode={shownMode}
        onPermissionMode={(permissionMode) => patch.mutate({ permissionMode })}
        readout={contextReadout(latestUsage(conv.rows))}
      />
    </div>
  );
}
