import { useCallback, useMemo, type ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { MessagesSquare } from 'lucide-react';
import { EmptyState, toast, type ApprovalAnswer } from '@ui';
import { cn } from '@ui/lib/utils';
import { api, ApiError, type SessionPatch } from '../api/client';
import type { Delivery } from '../api/gen/Delivery';
import type { Session } from '../api/gen/Session';
import type { MarkdownRefs } from '../common/markdownRefs';
import { actionError, runAction } from '../common/useAction';
import { latestUsage } from './blocks';
import { limitWarning } from './completion';
import { ConversationComposer } from './ConversationComposer';
import type { EventsSocketLike } from './eventsSocket';
import { useAgents } from '../common/agents';
import { MessageList } from './MessageList';
import { PinnedApprovalBar } from './PinnedApprovalBar';
import { useConversation, type ConversationData } from './useConversation';
import { useProjects } from '../projects/queries';
import { invalidateSessions, sessionKeys } from '../sessions/queries';
import { useHostSettings, useAgentCapabilities } from '../host/queries';

/** What Continue sends after an interrupted turn. */
const CONTINUE_PROMPT = 'Continue where you left off.';

const thousands = (n: number) => (n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));

/** "Context 45% · 90k / 200k" from the latest usage report. */
function contextReadout(usage: ReturnType<typeof latestUsage>): string | null {
  if (!usage?.contextUsed || !usage.contextWindow) return null;
  const pct = Math.round((usage.contextUsed / usage.contextWindow) * 100);
  return `Context ${pct}% · ${thousands(usage.contextUsed)} / ${thousands(usage.contextWindow)}`;
}

/** "Running in the background: Wait for CI": what the agent left running, which reports back in a turn of its own. */
function backgroundLine(tasks: { description: string }[]): string | null {
  if (tasks.length === 0) return null;
  const names = tasks.map((t) => t.description || 'a task').join(', ');
  return tasks.length === 1 ? `Running in the background: ${names}` : `${tasks.length} running in the background: ${names}`;
}

/** Background tasks a restart of PDT stopped: the agent hears of them with the next message (PDT-311). */
function stoppedLine(tasks: { description: string }[]): string | null {
  if (tasks.length === 0) return null;
  const names = tasks.map((t) => t.description || 'a task').join(', ');
  return `Background tasks stopped by the restart: ${names}. The agent is told with your next message.`;
}

/**
 * An agent session shown as a conversation: the message list over the composer. Sending also reopens a
 * parked, stopped or ended conversation, so there is no separate restart flow.
 */
export function ConversationView({ session, socketFactory, className, banner, readOnly }: {
  session: Session;
  /** Tests: a fake WebSocket. */
  socketFactory?: (url: string) => EventsSocketLike;
  className?: string;
  /** A quiet line over the composer. */
  banner?: ReactNode;
  /** Shown instead of the composer: the conversation is only to read (an ended PM session). */
  readOnly?: ReactNode;
}) {
  const conv = useConversation(session.id, socketFactory);
  return <ConversationPane session={session} conv={conv} className={className} banner={banner} readOnly={readOnly} />;
}

/** The conversation for events the caller follows (SessionView shares one socket between its header and this). */
export function ConversationPane({ session, conv, banner, readOnly, className }: {
  session: Session;
  conv: ConversationData;
  /** A quiet line over the composer (e.g. "Also open in the terminal"). */
  banner?: ReactNode;
  /** Shown instead of the composer. */
  readOnly?: ReactNode;
  className?: string;
}) {
  const qc = useQueryClient();
  const state = conv.state;
  const turnRunning = state?.turnRunning ?? false;
  const agent = session.agent ?? 'claude';
  const agentName = useAgents().label(agent);
  const caps = useAgentCapabilities(agent);
  const hostSettings = useHostSettings();
  const projects = useProjects();
  const project = projects.data?.find((p) => p.id === session.projectId) ?? null;
  // Ticket keys, PRs, the checkout's file paths and SHAs show as pills and links (PDT-382).
  const refs = useMemo<MarkdownRefs>(() => ({
    sessionId: project ? session.id : null,
    projectKey: project?.key ?? null,
    worktreeId: session.worktreeId,
    forge: project?.forge ?? null,
  }), [project, session.id, session.worktreeId]);

  const refreshSessions = () => Promise.all([
    invalidateSessions(qc, session.projectId),
    qc.invalidateQueries({ queryKey: sessionKeys.one(session.id) }),
  ]);
  const patch = useMutation({
    mutationFn: (p: SessionPatch) => api.updateSession(session.id, p),
    onSuccess: () => void refreshSessions(),
  });
  const shownModel = patch.isPending && patch.variables.model !== undefined ? patch.variables.model : session.model;
  const shownMode =
    patch.isPending && patch.variables.permissionMode
      ? patch.variables.permissionMode
      : state?.permissionMode ?? session.permissionMode ?? 'ask';

  const send = async (text: string, delivery?: Delivery, images?: string[]) => {
    await api.sendMessage(session.id, { text, ...(delivery ? { delivery } : {}), ...(images?.length ? { images } : {}) });
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
  const usage = latestUsage(conv.rows);
  const background = backgroundLine(state?.backgroundTasks ?? []);
  const stopped = stoppedLine(state?.stoppedBackground ?? []);
  const searchFiles = useCallback((q: string) => api.sessionFiles(session.id, q), [session.id]);
  const upload = useCallback((file: File) => api.uploadAttachment(session.id, file), [session.id]);
  const imageUrl = useCallback((id: string) => api.attachmentUrl(session.id, id), [session.id]);
  // Never automatic: a turn cut off by a restart or a crash waits for this click.
  const continueTurn = () => runAction(() => send(CONTINUE_PROMPT));

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <MessageList
        rows={conv.rows}
        provisional={conv.provisional}
        streams={conv.streams}
        turnRunning={turnRunning}
        pending={state?.pending ?? []}
        onAnswer={answer}
        imageUrl={imageUrl}
        agent={agent}
        onContinue={continueTurn}
        refs={refs}
        remember={`conversation:${session.id}`}
        empty={
          conv.ready ? (
            <EmptyState icon={<MessagesSquare />} className="mt-16 border-0">
              Ask {agentName} something. It works in {session.cwd}.
            </EmptyState>
          ) : null
        }
      />
      {banner}
      {background && (
        <p className="truncate px-4 pb-1 text-center text-meta text-muted-foreground" role="status" title={background}>
          {background}
        </p>
      )}
      {stopped && (
        <p className="truncate px-4 pb-1 text-center text-meta text-muted-foreground" role="status" title={stopped}>
          {stopped}
        </p>
      )}
      {conv.status === 'reconnecting' && (
        <p className="px-4 pb-1 text-center text-meta text-muted-foreground" role="status">Reconnecting…</p>
      )}
      <PinnedApprovalBar rows={conv.rows} />
      {readOnly ?? <ConversationComposer
        key={session.id}
        draftKey={session.id}
        turnRunning={turnRunning}
        onSend={send}
        onInterrupt={interrupt}
        stopped={state?.stopped ?? null}
        agentName={agentName}
        models={caps.data?.models ?? []}
        model={shownModel ?? null}
        defaultModel={hostSettings.data?.settings.agentModels?.[agent] ?? null}
        onModel={(model) => patch.mutate({ model })}
        permissionModes={caps.data?.permissionModes ?? []}
        permissionMode={shownMode}
        onPermissionMode={(permissionMode) => patch.mutate({ permissionMode })}
        readout={[state?.billing, contextReadout(usage)].filter(Boolean).join(' · ') || null}
        limit={limitWarning(usage?.rateLimits ?? [], Date.now())}
        commands={caps.data?.commands ?? []}
        terminalCommands={caps.data?.terminalCommands ?? []}
        onSearchFiles={searchFiles}
        onUpload={upload}
      />}
    </div>
  );
}
