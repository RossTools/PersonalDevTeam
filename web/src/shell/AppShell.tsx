import { useEffect, useRef, useState } from 'react';
import { PanelBottom } from 'lucide-react';
import { Button } from '@ui';
import { AddProjectForm } from '../projects/AddProjectForm';
import { ProjectSettingsPanel } from '../runs/ProjectSettingsPanel';
import { RunView } from '../runs/RunView';
import { SessionView } from '../sessions/SessionView';
import { FirstRunChecklist } from '../host/FirstRunChecklist';
import { HostSettingsPage } from '../host/HostSettingsPage';
import { UpdateBanner } from '../host/UpdateBanner';
import { KeyHelp } from '../workspaces/KeyHelp';
import { usePrefixKeys, type KeyAction } from '../workspaces/keys';
import { nextNeedingYou } from '../workspaces/model';
import { NewTabChooser } from '../workspaces/NewTabChooser';
import { ProjectTree } from '../workspaces/ProjectTree';
import { useWorkspaces } from '../workspaces/useWorkspaces';
import { WorkspaceBar } from '../workspaces/WorkspaceBar';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { VIEWS, type ViewId } from './views';

const PHASE_NOTE: Record<Exclude<ViewId, 'sessions' | 'settings'>, string> = {
  inbox: 'The inbox arrives with the pipeline (Phase 7).',
  board: 'The board arrives with tickets (Phase 4).',
  personas: 'Personas arrive with the pipeline (Phase 7).',
  files: 'The folder view arrives with git integration (Phase 3b).',
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
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [chooserOpen, setChooserOpen] = useState(false);
  const [renamingTabId, setRenamingTabId] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const ws = useWorkspaces();
  const tabId = ws.tab?.id ?? null;

  // Picking a tab leaves project settings.
  useEffect(() => setSettingsProjectId(null), [tabId]);

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
      case 'switcher': setSwitcherOpen(true); break;
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
  const bottomProject = ws.projects.find((p) => p.id === (current?.projectId ?? ws.workspace?.projectId)) ?? ws.projects[0] ?? null;

  return (
    <div className="h-full overflow-hidden flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 max-h-[40vh] md:max-h-none border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              <FirstRunChecklist onOpenSettings={() => setView('settings')} />
              <ProjectTree ws={ws} onOpenSettings={setSettingsProjectId} />
              <AddProjectForm />
            </>
          ) : view === 'settings' ? (
            <FirstRunChecklist />
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="relative flex-1 flex flex-col min-w-0 min-h-0">
        <UpdateBanner />
        {view !== 'settings' && (
          <WorkspaceBar
            ws={ws}
            switcherOpen={switcherOpen}
            setSwitcherOpen={setSwitcherOpen}
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
            {view === 'settings' ? (
              <HostSettingsPage />
            ) : settingsProject ? (
              <ProjectSettingsPanel project={settingsProject} />
            ) : current?.run ? (
              <RunView key={current.run.id} run={current.run} polledAt={current.polledAt} onRemoved={() => void ws.refresh()} />
            ) : current?.session ? (
              <SessionView key={current.session.id} session={current.session} polledAt={current.polledAt} onRemoved={() => void ws.refresh()} />
            ) : ws.projects.length === 0 ? (
              <p className="p-4 text-sm text-muted">Add a project to get started.</p>
            ) : (
              <p className="p-4 text-sm text-muted">
                {ws.tab ? 'Loading…' : 'No tabs here. Start a shell or an agent from the list, or press + (Ctrl+] c).'}
              </p>
            )}
          </div>
        )}
        {bottomOpen && <BottomPanel project={bottomProject} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
    </div>
  );
}
