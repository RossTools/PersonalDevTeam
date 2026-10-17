import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { PanelBottom } from 'lucide-react';
import { api } from '../api/client';
import { branchUrl } from '../projects/github';
import { Button } from '@ui';
import { AddProjectForm } from '../projects/AddProjectForm';
import { ProjectPage } from '../projects/ProjectPage';
import { FileViewer, FolderTree, useFolderView } from '../files/FolderView';
import { RunView } from '../runs/RunView';
import { SessionView } from '../sessions/SessionView';
import { HostSettingsPage } from '../host/HostSettingsPage';
import { UpdateBanner } from '../host/UpdateBanner';
import { KeyHelp } from '../workspaces/KeyHelp';
import { usePrefixKeys, type KeyAction } from '../workspaces/keys';
import { nextNeedingYou } from '../workspaces/model';
import { NewTabChooser } from '../workspaces/NewTabChooser';
import { Sidebar } from '../workspaces/Sidebar';
import { useWorkspaces } from '../workspaces/useWorkspaces';
import { WorkspaceBar } from '../workspaces/WorkspaceBar';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { VIEWS, type ViewId } from './views';

const PHASE_NOTE: Record<Exclude<ViewId, 'sessions' | 'settings' | 'files'>, string> = {
  inbox: 'The inbox arrives with the pipeline (Phase 7).',
  board: 'The board arrives with tickets (Phase 4).',
  personas: 'Personas arrive with the pipeline (Phase 7).',
};

/** Keeps the browser tab's title as `(<n>) <page title>` while n sessions need the user. */
function useAttentionTitle(count: number) {
  const base = useRef(document.title);
  useEffect(() => {
    document.title = count > 0 ? `(${count}) ${base.current}` : base.current;
  }, [count]);
  useEffect(() => () => { document.title = base.current; }, []);
}

export function AppShell() {
  const [view, setView] = useState<ViewId>('sessions');
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  const [settingsProjectId, setSettingsProjectId] = useState<string | null>(null);
  const [chooserOpen, setChooserOpen] = useState(false);
  const [renamingTabId, setRenamingTabId] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const ws = useWorkspaces();
  const tabId = ws.tab?.id ?? null;


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

  usePrefixKeys((a: KeyAction) => {
    const w = ws.workspace;
    const t = ws.tab;
    setView('sessions');
    switch (a.type) {
      case 'newTab': setChooserOpen(true); break;
      case 'switcher':
        // Focus the current workspace in the sidebar; Up/Down and Enter pick another.
        requestAnimationFrame(() => document.querySelector<HTMLElement>('#workspace-list [aria-current=true], #workspace-list [data-row]')?.focus());
        break;
      case 'help': setHelpOpen(true); break;
      case 'tab': if (w?.tabs[a.index]) ws.selectTab(w.tabs[a.index]); break;
      case 'next':
      case 'prev':
        if (w && t && w.tabs.length > 0) {
          const by = a.type === 'next' ? 1 : -1;
          ws.selectTab(w.tabs[(t.position + by + w.tabs.length) % w.tabs.length]);
        }
        break;
      case 'close': if (t) void ws.closeTab(t); break;
      case 'rename': if (t?.sessionId) setRenamingTabId(t.id); break;
      case 'needsYou': {
        const next = nextNeedingYou(ws.workspaces, t?.id ?? null, (item) => ws.info(item)?.activity === 'needs_you');
        if (next) ws.selectTab(next);
        break;
      }
    }
  });

  useAttentionTitle(ws.allInfos.filter((i) => i.activity === 'needs_you').length);

  const label = VIEWS.find((v) => v.id === view)!.label;
  const current = ws.currentInfo;
  const settingsProject = ws.projects.find((p) => p.id === settingsProjectId) ?? null;
  const currentProject = ws.projects.find((p) => p.id === current?.projectId) ?? null;
  const worktrees = useQuery({
    queryKey: ['worktrees', currentProject?.id ?? ''],
    queryFn: () => api.listWorktrees(currentProject!.id),
    enabled: !!currentProject?.githubRepo && !!current?.worktreeId,
  });
  const branch = current?.run?.branch ?? worktrees.data?.find((w) => w.id === current?.worktreeId)?.branch ?? null;
  const branchLink = currentProject?.githubRepo && branch ? branchUrl(currentProject.githubRepo, branch) : null;
  const folder = useFolderView(ws.projects, current ? { projectId: current.projectId, worktreeId: current.worktreeId } : null);
  const bottomProject = ws.projects.find((p) => p.id === (current?.projectId ?? ws.workspace?.projectId)) ?? ws.projects[0] ?? null;

  return (
    <div className="h-full overflow-hidden flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 max-h-[40vh] md:max-h-none border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              {ws.projects.length === 0 && (
                <p className="px-2 py-2 text-xs text-muted">
                  No projects yet. <button type="button" className="underline" onClick={() => setView('settings')}>Add one in Settings</button>.
                </p>
              )}
              <Sidebar ws={ws} onRenameTab={setRenamingTabId} />
            </>
          ) : view === 'files' ? (
            <FolderTree state={folder} />
          ) : view === 'settings' ? (
            <>
              <nav aria-label="Settings sections" className="flex flex-col gap-0.5 py-1">
                <button type="button" aria-current={settingsProjectId === null ? 'true' : undefined} onClick={() => setSettingsProjectId(null)}
                  className={`text-left px-2 py-1 rounded-pdt text-sm ${settingsProjectId === null ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}>This host</button>
                <h3 className="px-2 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">Projects</h3>
                {ws.projects.map((p) => (
                  <button key={p.id} type="button" title={p.repoPath} aria-current={settingsProjectId === p.id ? 'true' : undefined} onClick={() => setSettingsProjectId(p.id)}
                    className={`text-left px-2 py-1 rounded-pdt text-sm truncate ${settingsProjectId === p.id ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}>
                    <span className="mr-2 font-mono text-[11px] text-muted">{p.key}</span>{p.name}
                  </button>
                ))}
              </nav>
              <AddProjectForm onAdded={(p) => setSettingsProjectId(p.id)} />
            </>
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="relative flex-1 flex flex-col min-w-0 min-h-0">
        <UpdateBanner />
        {view !== 'settings' && view !== 'files' && (
          <WorkspaceBar
            ws={ws}
            renamingTabId={renamingTabId}
            setRenamingTabId={setRenamingTabId}
            onNewTab={() => setChooserOpen(true)}
            trailing={
              <Button variant="ghost" aria-label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
                <PanelBottom size={16} aria-hidden />
              </Button>
            }
          />
        )}
        {chooserOpen && view !== 'settings' && <NewTabChooser ws={ws} onClose={() => setChooserOpen(false)} />}
        {helpOpen && <KeyHelp onClose={() => setHelpOpen(false)} />}
        {!(bottomOpen && bottomMax) && (
          <div className="flex-1 flex flex-col min-h-0">
            {view === 'files' ? (
              <FileViewer state={folder} />
            ) : view === 'settings' ? (
              settingsProject ? <ProjectPage key={settingsProject.id} project={settingsProject} onRemoved={() => setSettingsProjectId(null)} /> : <HostSettingsPage />
            ) : current?.run ? (
              <RunView key={current.run.id} run={current.run} polledAt={current.polledAt} onRemoved={() => void ws.refresh()} branchLink={branchLink} />
            ) : current?.session ? (
              <SessionView key={current.session.id} session={current.session} polledAt={current.polledAt} onRemoved={() => void ws.refresh()} branchLink={branchLink} />
            ) : ws.projects.length === 0 ? (
              <p className="p-4 text-sm text-muted">Add a project to get started.</p>
            ) : (
              <p className="p-4 text-sm text-muted">
                {ws.tab ? 'Loading…' : 'No tabs here. Press + to start an agent or a shell (Ctrl+] c).'}
              </p>
            )}
          </div>
        )}
        {bottomOpen && <BottomPanel project={bottomProject} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
    </div>
  );
}
