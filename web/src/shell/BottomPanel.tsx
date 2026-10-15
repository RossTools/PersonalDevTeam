import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Maximize2, Minimize2, Plus } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { TerminalView } from '../terminal/TerminalView';

export function BottomPanel({ project, maximized, onToggleMaximize }: {
  project: Project | null;
  maximized: boolean;
  onToggleMaximize: () => void;
}) {
  const qc = useQueryClient();
  const sessions = useQuery({
    queryKey: ['sessions', project?.id ?? null],
    queryFn: () => api.listSessions(project!.id),
    enabled: project !== null,
    refetchInterval: 2000,
  });
  const shells = (sessions.data ?? []).filter((s) => s.kind === 'shell' && s.status === 'running');
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = shells.find((s) => s.id === activeId) ?? shells[0] ?? null;
  const create = useMutation({
    mutationFn: () => api.createSession({ projectId: project!.id, kind: 'shell', cols: 120, rows: 12 }),
    onSuccess: (s) => {
      setActiveId(s.id);
      void qc.invalidateQueries({ queryKey: ['sessions', project!.id] });
    },
  });

  return (
    <section aria-label="Terminal panel" className={`${maximized ? 'flex-1' : 'h-64'} border-t border-border bg-surface flex flex-col min-h-0`}>
      <div className="flex items-center gap-1 px-2 py-1 border-b border-border text-xs">
        <div role="tablist" aria-label="Terminals" className="flex items-center gap-1 overflow-x-auto">
          {shells.map((s, i) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              aria-selected={s.id === active?.id}
              onClick={() => setActiveId(s.id)}
              className={`rounded-pdt px-2 py-0.5 ${s.id === active?.id ? 'bg-accent-soft text-fg' : 'text-muted hover:bg-surface-2'}`}
            >
              {s.title} {i + 1}
            </button>
          ))}
        </div>
        <Button variant="ghost" aria-label="New shell" disabled={!project || create.isPending} onClick={() => create.mutate()}>
          <Plus size={14} aria-hidden />
        </Button>
        <span className="flex-1" />
        <Button variant="ghost" aria-label={maximized ? 'Restore terminal panel' : 'Maximize terminal panel'} onClick={onToggleMaximize}>
          {maximized ? <Minimize2 size={14} aria-hidden /> : <Maximize2 size={14} aria-hidden />}
        </Button>
      </div>
      {create.isError && <p role="alert" className="px-3 py-1 text-xs text-failed">{create.error.message}</p>}
      {!project ? (
        <p className="p-3 text-xs text-muted">Select a project to open a terminal.</p>
      ) : active ? (
        <TerminalView key={active.id} sessionId={active.id} />
      ) : (
        <p className="p-3 text-xs text-muted">No shells running. Press + to start one.</p>
      )}
    </section>
  );
}
