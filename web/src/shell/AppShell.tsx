import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { PanelBottom } from 'lucide-react';
import { api } from '../api/client';
import { branchUrl } from '../projects/github';
import { Button, EmptyState, IconButton, List, ListRow } from '@ui';
import { ProjectKey } from '../common/BranchChip';
import { runAction } from '../common/useAction';
import { AddProjectForm } from '../projects/AddProjectForm';
import { AddProjectDialog } from '../projects/AddProjectDialog';
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
  const [addingProject, setAddingProject] = useState(false);
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
        requestAnimationFrame(() => document.querySelector<HTMLElement>('#workspace-list [data-current], #workspace-list [role=row]')?.focus());
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
      case 'close': if (t) void runAction(() => ws.closeTab(t)); break;
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
    <div className="flex h-full flex-col-reverse overflow-hidden bg-background text-foreground md:flex-row">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 max-h-[40vh] md:max-h-none border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-2xs uppercase tracking-wider text-muted-foreground">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              {ws.projects.length === 0 && (
                <EmptyState action={<Button size="sm" onClick={() => setView('settings')}>Add one in Settings</Button>}>
                  No projects yet.
                </EmptyState>
              )}
              <Sidebar
                ws={ws}
                onRenameTab={setRenamingTabId}
                onProjectSettings={(id) => { setSettingsProjectId(id); setView('settings'); }}
                onAddProject={() => setAddingProject(true)}
              />
            </>
          ) : view === 'files' ? (
            <FolderTree state={folder} />
          ) : view === 'settings' ? (
            <>
              <nav aria-label="Settings sections" className="flex flex-col gap-0.5 py-1">
                <List aria-label="Host" onAction={() => setSettingsProjectId(null)}>
                  <ListRow id="host" title="This host" current={settingsProjectId === null} />
                </List>
                <h3 className="px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase">Projects</h3>
                <List aria-label="Projects" onAction={setSettingsProjectId}>
                  {ws.projects.map((p) => (
                    <ListRow key={p.id} id={p.id} title={p.name} textValue={p.name} current={settingsProjectId === p.id}
                      leading={<ProjectKey projectKey={p.key} />} />
                  ))}
                </List>
              </nav>
              <AddProjectForm onAdded={(p) => setSettingsProjectId(p.id)} />
            </>
          ) : (
            <EmptyState>{PHASE_NOTE[view]}</EmptyState>
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
              <IconButton label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
                <PanelBottom aria-hidden />
              </IconButton>
            }
          />
        )}
        {chooserOpen && view !== 'settings' && <NewTabChooser ws={ws} onClose={() => setChooserOpen(false)} />}
        {helpOpen && <KeyHelp onClose={() => setHelpOpen(false)} />}
        {addingProject && (
          <AddProjectDialog
            onClose={() => setAddingProject(false)}
            onAdded={(p) => {
              // Show the new project's workspace once the list includes it.
              void runAction(async () => {
                const list = await api.listWorkspaces();
                const home = list.find((w) => w.projectId === p.id);
                if (home) ws.selectWorkspace(home.id);
                await ws.refresh();
              });
            }}
          />
        )}
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
              <EmptyState>Add a project to get started.</EmptyState>
            ) : (
              <EmptyState>{ws.tab ? 'Loading…' : 'No tabs here. Press + to start an agent or a shell (Ctrl+] c).'}</EmptyState>
            )}
          </div>
        )}
        {bottomOpen && <BottomPanel project={bottomProject} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
    </div>
  );
}
