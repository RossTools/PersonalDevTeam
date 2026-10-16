import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { PanelBottom } from 'lucide-react';
import { Button } from '@ui';
import { api } from '../api/client';
import { ProjectsList } from '../projects/ProjectsList';
import { AddProjectForm } from '../projects/AddProjectForm';
import { SessionList } from '../sessions/SessionList';
import { SessionView } from '../sessions/SessionView';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { VIEWS, type ViewId } from './views';

const PHASE_NOTE: Record<Exclude<ViewId, 'sessions'>, string> = {
  inbox: 'The inbox arrives with the pipeline (Phase 5).',
  board: 'The board arrives with tickets (Phase 2).',
  personas: 'Personas arrive with the pipeline (Phase 5).',
  files: 'The file view arrives with review (Phase 4).',
};

export function AppShell() {
  const [view, setView] = useState<ViewId>('sessions');
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === '`') {
        e.preventDefault();
        setBottomOpen((o) => !o);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const project = projects.data?.find((p) => p.id === projectId) ?? null;
  const sessions = useQuery({
    queryKey: ['sessions', projectId],
    queryFn: () => api.listSessions(projectId!),
    enabled: projectId !== null,
    refetchInterval: 2000,
  });
  const session = sessions.data?.find((s) => s.id === sessionId) ?? null;
  const selectProject = (id: string) => {
    setProjectId(id);
    setSessionId(null);
  };
  const label = VIEWS.find((v) => v.id === view)!.label;

  return (
    <div className="h-full flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 max-h-[40vh] md:max-h-none border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              <ProjectsList selectedId={projectId} onSelect={selectProject} />
              {project && <SessionList project={project} selectedId={sessionId} onSelect={setSessionId} />}
              <AddProjectForm />
            </>
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="flex items-center gap-2 border-b border-border px-3 py-1 text-sm">
          <span className="flex-1 truncate text-muted">{project ? `Project ${project.name}` : 'Select a project to get started.'}</span>
          <Button variant="ghost" aria-label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
            <PanelBottom size={16} aria-hidden />
          </Button>
        </div>
        {!(bottomOpen && bottomMax) && (
          <div className="flex-1 flex flex-col min-h-0">
            {session ? (
              <SessionView session={session} polledAt={sessions.dataUpdatedAt} onRemoved={() => setSessionId(null)} />
            ) : (
              project && <p className="p-4 text-sm text-muted">Start a shell or an agent from the list.</p>
            )}
          </div>
        )}
        {bottomOpen && <BottomPanel project={project} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
    </div>
  );
}
