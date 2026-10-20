import { lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, PanelBottom, Plus } from 'lucide-react';
import { api } from '../api/client';
import { branchUrl } from '../projects/github';
import { Button, EmptyState, IconButton, List, ListRow, Sheet, Skeleton, useThemePref } from '@ui';
import { ProjectKey } from '../common/BranchChip';
import type { Place } from '../common/PlacePicker';
import { runAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { CommandPalette } from '../palette/CommandPalette';
import { paletteItems, type NewKind, type PaletteHandlers } from '../palette/paletteItems';
import { useMetaK } from '../palette/useMetaK';
import { AddProjectDialog } from '../projects/AddProjectDialog';
import { newSession, type Choice } from '../sessions/newSession';
import { ProjectPage } from '../projects/ProjectPage';
import { FileViewer, FolderTree, useFolderView } from '../files/FolderView';
import { RunView } from '../runs/RunView';
import { SessionView } from '../sessions/SessionView';
import { HostSettingsPage } from '../host/HostSettingsPage';
import { OpenSessionContext, OpenTicketContext, StartTicketContext, startNextPart } from '../tickets/actions';
import { invalidateTickets } from '../tickets/queries';
import type { StartResult } from '../api/gen/StartResult';
import type { TabPlacement } from '../api/gen/TabPlacement';
import { looksLikeKey } from '../tickets/model';
import { useTicketSearch } from '../tickets/queries';
import { TicketsList } from '../tickets/TicketsList';
import { useBacklog } from '../tickets/useBacklog';
import { UpdateBanner } from '../host/UpdateBanner';
import { useReloadOnNewBuild } from '../host/useReloadOnNewBuild';
import { KeyHelp } from '../workspaces/KeyHelp';
import { usePrefixKeys, type KeyAction } from '../workspaces/keys';
import { nextNeedingYou } from '../workspaces/model';
import { NewTabChooser } from '../workspaces/NewTabChooser';
import { Sidebar } from '../workspaces/Sidebar';
import { useWorkspaces } from '../workspaces/useWorkspaces';
import { WorkspaceBar } from '../workspaces/WorkspaceBar';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { InstanceBadge } from './InstanceBadge';
import { useDevTitle } from './useInstance';
import { VIEWS, type ViewId } from './views';

const CHOICE: Record<NewKind, Choice> = { claude: 'Claude', claude_terminal: 'Claude (terminal)', codex: 'Codex', codex_terminal: 'Codex (terminal)', devin_terminal: 'Devin (terminal)', shell: 'Shell' };

/** The list column's name on a phone, where it is a sheet: the opener's label and the sheet's title. */
const LIST_NAME: Record<ViewId, string> = {
  sessions: 'Workspaces and agents', tickets: 'Projects and filters', files: 'Files and folders', settings: 'Settings sections',
};

const listHeading = 'px-3 pt-3 pb-1 text-2xs uppercase tracking-wider text-muted-foreground';

/** Keeps the browser tab's title as `(<n>) [dev] <page title>`: n sessions need the user; [dev] on a dev instance. */
function useAttentionTitle(count: number) {
  useDevTitle(count);
  useReloadOnNewBuild();
}

// The ticket screens (and the Markdown libraries they use) load the first time they're shown.
const TicketsView = lazy(() => import('../tickets/TicketsView').then((m) => ({ default: m.TicketsView })));
const NewTicketDialog = lazy(() => import('../tickets/NewTicketDialog').then((m) => ({ default: m.NewTicketDialog })));
const TicketView = lazy(() => import('../tickets/TicketView').then((m) => ({ default: m.TicketView })));
const StartDialog = lazy(() => import('../tickets/StartDialog').then((m) => ({ default: m.StartDialog })));

export function AppShell() {
  const [view, setView] = useState<ViewId>('sessions');
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  const [settingsProjectId, setSettingsProjectId] = useState<string | null>(null);
  const [chooserOpen, setChooserOpen] = useState(false);
  const [renamingTabId, setRenamingTabId] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [addingProject, setAddingProject] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  // The New ticket form: its project and the title typed so far.
  const [newTicket, setNewTicket] = useState<{ projectId: string | null; title: string } | null>(null);
  // The Start dialog: the ticket, and where its tabs open (undefined: the end of the project workspace).
  const [starting, setStarting] = useState<{ ticketId: string; placement?: TabPlacement } | null>(null);
  // Phones: the list column is a sheet from the left, opened from the header.
  const [sheetOpen, setSheetOpen] = useState(false);
  const phone = useMediaQuery(PHONE_QUERY);
  const plusRef = useRef<HTMLButtonElement>(null);
  const [theme, setTheme] = useThemePref();
  const qc = useQueryClient();
  const ws = useWorkspaces();
  const backlog = useBacklog();
  const openTicket = useCallback((t: { id: string; projectId: string }) => {
    setView('sessions');
    void runAction(() => ws.focusItem({ kind: 'ticket', id: t.id }, t.projectId));
  }, [ws.focusItem]);
  const openSession = useCallback((s: { id: string; projectId: string }) => {
    setView('sessions');
    void runAction(() => ws.focusItem({ kind: 'session', id: s.id }, s.projectId));
  }, [ws.focusItem]);
  // Started from the ticket's own tab: the tabs open where it is (it keeps its place, the session goes after it);
  // otherwise the server puts them at the end of the project workspace.
  const startTicket = useCallback((t: { id: string; projectId: string }, opts?: { here?: boolean }) => {
    const w = ws.workspace;
    const cur = ws.tab;
    let placement: TabPlacement | undefined;
    if (opts?.here && w) {
      placement = { workspaceId: w.id, position: cur ? (cur.ticketId === t.id ? cur.position : cur.position + 1) : w.tabs.length };
    }
    setStarting({ ticketId: t.id, ...(placement ? { placement } : {}) });
  }, [ws.workspace, ws.tab]);
  const onStarted = (r: StartResult) => {
    invalidateTickets(qc, [r.ticket.id]);
    qc.setQueryData(['ticket', r.ticket.id], r.ticket);
    void qc.invalidateQueries({ queryKey: ['sessions', r.session.projectId] });
    void qc.invalidateQueries({ queryKey: ['worktrees', r.session.projectId] });
    setView('sessions');
    void runAction(async () => {
      await ws.refresh();
      await ws.focusItem({ kind: 'session', id: r.session.id }, r.session.projectId);
    });
  };
  const keyQuery = paletteQuery.trim();
  const ticketHits = useTicketSearch({ q: keyQuery, limit: 5 }, paletteOpen && looksLikeKey(keyQuery));

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

  useMetaK(() => setPaletteOpen(true));

  usePrefixKeys((a: KeyAction) => {
    const w = ws.workspace;
    const t = ws.tab;
    if (a.type === 'palette') {
      setPaletteOpen(true);
      return;
    }
    if (a.type === 'backlog' || a.type === 'board') {
      setView('tickets');
      backlog.setMode(a.type);
      return;
    }
    setView('sessions');
    switch (a.type) {
      case 'newTab': setChooserOpen(true); break;
      case 'switcher':
        // Focus the current workspace in the sidebar; Up/Down and Enter pick another. On a phone, open the sheet first.
        if (phone) setSheetOpen(true);
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
    enabled: !!currentProject && !!current?.worktreeId,
  });
  const worktreeBranch = current?.worktreeId ? worktrees.data?.find((w) => w.id === current.worktreeId)?.branch : undefined;
  const branch = current?.run?.branch ?? worktreeBranch ?? null;
  const branchLink = currentProject?.githubRepo && branch ? branchUrl(currentProject.githubRepo, branch) : null;
  const folder = useFolderView(ws.projects, current ? { projectId: current.projectId, worktreeId: current.worktreeId } : null);
  const bottomProject = ws.projects.find((p) => p.id === (current?.projectId ?? ws.workspace?.projectId)) ?? ws.projects[0] ?? null;

  // Choosing a file closes the phone sheet.
  useEffect(() => {
    if (folder.file) setSheetOpen(false);
  }, [folder.file]);

  const showProjectSettings = (id: string | null) => {
    setSettingsProjectId(id);
    setView('settings');
    setSheetOpen(false);
  };

  // The palette's items, built only while it is open (the screen re-renders every 2 s).
  const place: Place | null = current
    ? { projectId: current.projectId, worktreeId: current.worktreeId }
    : ws.workspace?.projectId ? { projectId: ws.workspace.projectId, worktreeId: null } : null;
  // New tickets go in the backlog's project in the Tickets view, else the current workspace's project.
  const ticketProjectId = view === 'tickets' ? backlog.projectId : place?.projectId ?? backlog.projectId;
  const handlers: PaletteHandlers = {
    selectWorkspace: (id) => { setView('sessions'); ws.selectWorkspace(id); },
    selectTab: (t) => { setView('sessions'); ws.selectTab(t); },
    focusItem: (item, projectId) => { setView('sessions'); void runAction(() => ws.focusItem(item, projectId)); },
    projectSettings: (id) => showProjectSettings(id),
    hostSettings: () => showProjectSettings(null),
    newSession: (kind, at) => {
      setView('sessions');
      const w = ws.workspace;
      // Like the new-tab chooser: right after the current tab.
      const tab = w ? { workspaceId: w.id, position: ws.tab ? ws.tab.position + 1 : w.tabs.length } : undefined;
      void runAction(async () => {
        const s = await api.createSession({ ...newSession(at.projectId, at.worktreeId, CHOICE[kind]), ...(tab ? { tab } : {}) });
        void qc.invalidateQueries({ queryKey: ['sessions', s.projectId] });
        await ws.refresh();
        await ws.focusItem({ kind: 'session', id: s.id }, s.projectId);
      });
    },
    openChooser: () => { setView('sessions'); setChooserOpen(true); },
    renameTab: (t) => { setView('sessions'); ws.selectTab(t); setRenamingTabId(t.id); },
    closeTab: (t) => void runAction(() => ws.closeTab(t)),
    moveTab: (t, workspaceId) => void runAction(() => ws.moveTab(t, workspaceId)),
    moveTabToNewWorkspace: (t, name) => void runAction(async () => {
      const created = await api.createWorkspace({ name });
      await ws.moveTab(t, created.id);
      ws.selectWorkspace(created.id);
    }),
    setTheme,
    showKeys: () => setHelpOpen(true),
    showFiles: () => setView('files'),
    newTicket: () => setNewTicket({ projectId: ticketProjectId, title: '' }),
    openTicket,
    startTicket: (t, here) => startTicket(t, { here }),
    startNextPart: (t) => void runAction(() => startNextPart(startTicket, t)),
  };
  const items = paletteOpen
    ? paletteItems({
        workspaces: ws.workspaces, infos: [...ws.allInfos, ...ws.ticketInfos], projects: ws.projects, tab: ws.tab, place, theme, handlers,
        tickets: looksLikeKey(keyQuery) ? ticketHits.data?.items ?? [] : [],
        ticketProjectId: view === 'tickets' ? backlog.projectId : null,
      })
    : [];

  const list: ReactNode = view === 'sessions' ? (
    // The Sidebar has its own section headings (Workspaces, Agents).
    <>
      {ws.projects.length === 0 && (
        <EmptyState action={<Button size="sm" onClick={() => { setView('settings'); setSheetOpen(false); }}>Add one in Settings</Button>}>
          No projects yet.
        </EmptyState>
      )}
      <Sidebar
        ws={ws}
        onRenameTab={setRenamingTabId}
        onProjectSettings={showProjectSettings}
        onChoose={() => setSheetOpen(false)}
      />
    </>
  ) : view === 'tickets' ? (
    <TicketsList backlog={backlog} projects={ws.projects} onChoose={() => setSheetOpen(false)} />
  ) : view === 'files' ? (
    <>
      <h2 className={listHeading}>{label}</h2>
      <FolderTree state={folder} />
    </>
  ) : (
    <nav aria-label="Settings sections" className="flex flex-col gap-0.5 py-1">
      <h2 className={listHeading}>{label}</h2>
      <List aria-label="Host" onAction={() => showProjectSettings(null)}>
        <ListRow id="host" title="This host" current={settingsProjectId === null} />
      </List>
      <h3 className="flex items-center px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase">
        <span className="flex-1">Projects</span>
        <IconButton label="Add project" size="sm" noTooltip className="size-5" onClick={() => { setSheetOpen(false); setAddingProject(true); }}>
          <Plus aria-hidden />
        </IconButton>
      </h3>
      <List aria-label="Projects" onAction={(id) => showProjectSettings(id)}>
        {ws.projects.map((p) => (
          <ListRow key={p.id} id={p.id} title={p.name} textValue={p.name} current={settingsProjectId === p.id}
            leading={<ProjectKey projectKey={p.key} />} />
        ))}
      </List>
    </nav>
  );
  const openList = () => setSheetOpen(true);

  return (
    <OpenTicketContext.Provider value={openTicket}>
    <OpenSessionContext.Provider value={openSession}>
    <StartTicketContext.Provider value={startTicket}>
    <div className="flex h-full flex-col-reverse overflow-hidden bg-background text-foreground md:flex-row">
      <IconRail
        active={view}
        onSelect={(v) => { setView(v); setSheetOpen(false); }}
        onSearch={phone ? () => setPaletteOpen(true) : undefined}
        top={phone ? undefined : <InstanceBadge stacked />}
      />
      {phone ? (
        <Sheet side="left" title={LIST_NAME[view]} hideTitle open={sheetOpen} onOpenChange={setSheetOpen} className="w-72 max-w-[85vw]">
          <div className="-mx-2">{list}</div>
        </Sheet>
      ) : (
        <aside aria-label={`${label} list`} className="flex min-h-0 w-72 flex-col border-r border-border">
          <div className="flex-1 overflow-auto px-1">{list}</div>
        </aside>
      )}
      <main className="relative flex-1 flex flex-col min-w-0 min-h-0">
        <UpdateBanner />
        {view === 'sessions' ? (
          <WorkspaceBar
            ws={ws}
            renamingTabId={renamingTabId}
            setRenamingTabId={setRenamingTabId}
            onNewTab={() => setChooserOpen(true)}
            plusRef={plusRef}
            {...(phone ? { onOpenList: openList, leading: <InstanceBadge compact /> } : {})}
            trailing={
              <IconButton label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
                <PanelBottom aria-hidden />
              </IconButton>
            }
          />
        ) : phone ? (
          <div className="flex min-w-0 items-center gap-1 border-b border-border px-2 py-1">
            <Button variant="ghost" size="sm" aria-label={LIST_NAME[view]} aria-haspopup="dialog" onClick={openList} className="font-medium">
              {label} <ChevronDown aria-hidden className="size-4 text-muted-foreground" />
            </Button>
            <InstanceBadge compact />
          </div>
        ) : null}
        {chooserOpen && view === 'sessions' && <NewTabChooser ws={ws} onClose={() => setChooserOpen(false)} anchor={plusRef} />}
        {helpOpen && <KeyHelp onClose={() => setHelpOpen(false)} />}
        <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} items={items} onQueryChange={setPaletteQuery} />
        {newTicket && (
          <Suspense fallback={null}><NewTicketDialog
            projects={ws.projects}
            projectId={newTicket.projectId}
            title={newTicket.title}
            onClose={() => setNewTicket(null)}
          /></Suspense>
        )}
        {starting && (
          <Suspense fallback={null}><StartDialog
            ticketId={starting.ticketId}
            placement={starting.placement}
            onClose={() => setStarting(null)}
            onStarted={onStarted}
          /></Suspense>
        )}
        {addingProject && (
          <AddProjectDialog
            onClose={() => setAddingProject(false)}
            onAdded={(p) => {
              if (view === 'settings') {
                setSettingsProjectId(p.id);
                return;
              }
              // Show the new project's workspace once the list includes it.
              void runAction(async () => {
                const all = await api.listWorkspaces();
                const home = all.find((w) => w.projectId === p.id);
                if (home) ws.selectWorkspace(home.id);
                await ws.refresh();
              });
            }}
          />
        )}
        {!(bottomOpen && bottomMax) && (
          <div className="flex-1 flex flex-col min-h-0">
            {view === 'tickets' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><TicketsView backlog={backlog} projects={ws.projects} onNewTicket={(projectId, title) => setNewTicket({ projectId, title: title ?? '' })} /></Suspense>
            ) : view === 'files' ? (
              <FileViewer state={folder} />
            ) : view === 'settings' ? (
              settingsProject ? <ProjectPage key={settingsProject.id} project={settingsProject} onRemoved={() => setSettingsProjectId(null)} /> : <HostSettingsPage />
            ) : ws.tab?.ticketId ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><TicketView key={ws.tab.ticketId} ticketId={ws.tab.ticketId} onDeleted={() => void ws.refresh()} /></Suspense>
            ) : current?.run ? (
              <RunView
                key={current.run.id}
                run={current.run}
                polledAt={current.polledAt}
                onRemoved={() => void ws.refresh()}
                branchLink={branchLink}
                {...(currentProject ? { repo: currentProject.githubRepo, baseBranch: currentProject.baseBranch } : {})}
              />
            ) : current?.session ? (
              <SessionView
                key={current.session.id}
                session={current.session}
                polledAt={current.polledAt}
                onRemoved={() => void ws.refresh()}
                branchLink={branchLink}
                {...(currentProject ? { repo: currentProject.githubRepo, baseBranch: currentProject.baseBranch, projectKey: currentProject.key } : {})}
                {...(!current.worktreeId ? { branch: null } : worktreeBranch !== undefined ? { branch: worktreeBranch } : {})}
              />
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
    </StartTicketContext.Provider>
    </OpenSessionContext.Provider>
    </OpenTicketContext.Provider>
  );
}
