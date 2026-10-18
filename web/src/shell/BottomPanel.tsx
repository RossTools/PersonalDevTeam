import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Maximize2, Minimize2, Plus } from 'lucide-react';
import { useState } from 'react';
import { cn } from '@ui/lib/utils';
import { EmptyState, IconButton, SegmentedControl } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { useAction } from '../common/useAction';
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
  const [create, creating] = useAction(async () => {
    const s = await api.createSession({ projectId: project!.id, kind: 'shell', cols: 120, rows: 12 });
    setActiveId(s.id);
    void qc.invalidateQueries({ queryKey: ['sessions', project!.id] });
  });

  return (
    <section aria-label="Terminal panel" className={cn(maximized ? 'flex-1' : 'h-64', 'flex min-h-0 flex-col border-t border-border bg-card')}>
      <div className="flex items-center gap-1 border-b border-border px-2 py-1 text-xs">
        {shells.length > 0 && (
          <SegmentedControl
            aria-label="Terminals"
            size="sm"
            className="min-w-0 overflow-x-auto"
            value={active?.id ?? ''}
            onChange={setActiveId}
            options={shells.map((s, i) => ({ value: s.id, label: `${s.title} ${i + 1}` }))}
          />
        )}
        <IconButton label="New shell" size="sm" disabled={!project} pending={creating} onClick={() => void create()}>
          <Plus aria-hidden />
        </IconButton>
        <span className="flex-1" />
        <IconButton label={maximized ? 'Restore terminal panel' : 'Maximize terminal panel'} size="sm" onClick={onToggleMaximize}>
          {maximized ? <Minimize2 aria-hidden /> : <Maximize2 aria-hidden />}
        </IconButton>
      </div>
      {!project ? (
        <EmptyState>Select a project to open a terminal.</EmptyState>
      ) : active ? (
        <TerminalView key={active.id} sessionId={active.id} sessionStatus={active.status} polledAt={sessions.dataUpdatedAt} />
      ) : (
        <EmptyState>No shells running. Press + to start one.</EmptyState>
      )}
    </section>
  );
}
