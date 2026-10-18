import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Link, useConfirm, ViewHeader } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { agentState, AgentDot, AgentLabel } from '../common/AgentState';
import { TerminalView } from '../terminal/TerminalView';
import { INITIAL_SIZE } from './status';
import { useViewTracking } from './useViewTracking';

export function SessionView({ session, polledAt, onRemoved, branchLink }: {
  session: Session;
  polledAt?: number;
  onRemoved: () => void;
  /** The session's branch on GitHub, when known. */
  branchLink?: string | null;
}) {
  useViewTracking(session.id, polledAt);
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [composing, setComposing] = useState(false);
  const refresh = () => qc.invalidateQueries({ queryKey: ['sessions', session.projectId] });
  const kill = useMutation({ mutationFn: () => api.killSession(session.id), onSuccess: refresh });
  const restart = useMutation({ mutationFn: () => api.restartSession(session.id, INITIAL_SIZE), onSuccess: refresh });
  const remove = useMutation({
    mutationFn: () => api.deleteSession(session.id),
    onSuccess: () => {
      void refresh();
      onRemoved();
    },
  });
  const running = session.status === 'running';
  const error = kill.error ?? restart.error ?? remove.error;
  // One action at a time: the daemon serializes them anyway, and a second click would only fail.
  const busy = kill.isPending || restart.isPending || remove.isPending;
  const state = agentState({ kind: session.kind, status: session.status, activity: session.activity, unseen: session.unseen });

  const askRemove = async () => {
    const { ok } = await confirm({
      title: `Remove ${session.title}?`,
      body:
        session.kind === 'agent_tui'
          ? 'Its terminal history is deleted and it can no longer be restarted or resumed here. Files in its folder stay.'
          : 'Its terminal history is deleted and it can no longer be restarted. Files in its folder stay.',
      action: 'Remove session',
      danger: true,
    });
    if (ok) remove.mutate();
  };

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <ViewHeader
        leading={<AgentDot state={state} />}
        title={session.title}
        meta={
          <>
            <span className="min-w-0 truncate" title={session.cwd}>{session.cwd}</span>
            <AgentLabel state={state} className="shrink-0" />
            {branchLink && <Link href={branchLink} external className="shrink-0">GitHub</Link>}
          </>
        }
        actions={
          <>
            {running && (
              <Button variant="ghost" size="sm" aria-pressed={composing} onClick={() => setComposing((c) => !c)}>Compose</Button>
            )}
            {running ? (
              <Button variant="ghost" size="sm" disabled={busy} pending={kill.isPending} onClick={() => kill.mutate()}>Stop</Button>
            ) : (
              <>
                <Button variant="ghost" size="sm" disabled={busy} pending={restart.isPending} onClick={() => restart.mutate()}>Restart</Button>
                <Button variant="ghost" size="sm" disabled={busy} pending={remove.isPending} onClick={() => void askRemove()}>Remove</Button>
              </>
            )}
          </>
        }
      />
      {error && <div className="px-3 py-1.5"><Alert tone="error">{error.message}</Alert></div>}
      {session.status === 'interrupted' ? (
        <p className="p-4 text-sm text-muted-foreground">
          This session stopped when the daemon shut down. Restart it to continue
          {session.kind === 'agent_tui' ? '; the agent resumes its last conversation' : ''}.
        </p>
      ) : (
        <TerminalView
          key={session.id}
          sessionId={session.id}
          sessionStatus={session.status}
          polledAt={polledAt}
          composing={composing && running}
          onComposeClose={() => setComposing(false)}
        />
      )}
    </div>
  );
}
