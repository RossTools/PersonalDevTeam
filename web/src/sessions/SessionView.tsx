import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { TerminalView } from '../terminal/TerminalView';
import { INITIAL_SIZE, toDot } from './status';

export function SessionView({ session, onRemoved }: { session: Session; onRemoved: () => void }) {
  const qc = useQueryClient();
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

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <StatusDot status={toDot(session.status)} />
        <span className="font-medium">{session.title}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted" title={session.cwd}>{session.cwd}</span>
        {running ? (
          <Button variant="ghost" onClick={() => kill.mutate()}>Stop</Button>
        ) : (
          <>
            <Button variant="ghost" onClick={() => restart.mutate()}>Restart</Button>
            <Button variant="ghost" onClick={() => remove.mutate()}>Remove</Button>
          </>
        )}
      </header>
      {error && <p role="alert" className="px-3 py-1 text-xs text-failed">{error.message}</p>}
      {session.status === 'interrupted' ? (
        <p className="p-4 text-sm text-muted">
          This session stopped when the daemon shut down. Restart it to continue
          {session.kind === 'agent_tui' ? '; the agent resumes its last conversation' : ''}.
        </p>
      ) : (
        // Remount on status change so a restarted session gets a fresh socket.
        <TerminalView key={`${session.id}-${session.status}`} sessionId={session.id} />
      )}
    </div>
  );
}
