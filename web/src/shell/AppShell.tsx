import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { ProjectsList } from '../projects/ProjectsList';
import { AddProjectForm } from '../projects/AddProjectForm';
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
  const [projectId, setProjectId] = useState<string | null>(null);

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
  const selectedName = projects.data?.find((p) => p.id === projectId)?.name;

  const label = VIEWS.find((v) => v.id === view)!.label;

  return (
    <div className="h-full flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              <ProjectsList selectedId={projectId} onSelect={setProjectId} />
              <AddProjectForm />
            </>
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="flex-1 overflow-auto p-4 text-sm text-muted">
          {projectId ? `Project ${selectedName ?? ''}`.trim() : 'Select a project to get started.'}
        </div>
        {bottomOpen && <BottomPanel />}
      </main>
    </div>
  );
}
