import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, PanelBottom, Plus } from 'lucide-react';
import { api } from '../api/client';
import { branchUrl } from '../projects/github';
import { Button, EmptyState, IconButton, List, ListRow, Sheet, Skeleton, useConfirm, useThemePref, type MenuEntry } from '@ui';
import { ProjectKey } from '../common/BranchChip';
import type { Place } from '../common/PlacePicker';
import { runAction } from '../common/useAction';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { useKeyboardBox } from '../common/viewport';
import { CommandPalette } from '../palette/CommandPalette';
import { paletteItems, type NewKind, type PaletteHandlers } from '../palette/paletteItems';
import { useMetaK } from '../palette/useMetaK';
import { AddProjectDialog } from '../projects/AddProjectDialog';
import { newSession, type Choice } from '../sessions/newSession';
import { ProjectPage } from '../projects/ProjectPage';
import { FileViewer, FolderTree, useFolderView, type FolderViewState } from '../files/FolderView';
import { RunView } from '../runs/RunView';
import { SessionView } from '../sessions/SessionView';
import { HostSettingsPage } from '../host/HostSettingsPage';
import { ConnectionsPage } from '../connections/ConnectionsPage';
import { WorkflowsList, type WorkflowTarget } from '../workflows/WorkflowsList';
import { OpenSessionContext, OpenTicketContext, StartTicketContext, startNextPart } from '../tickets/actions';
import { invalidateTickets } from '../tickets/queries';
import { OpenChangesContext, type OpenChangesOptions } from '../review/open';
import type { StartResult } from '../api/gen/StartResult';
import type { TabPlacement } from '../api/gen/TabPlacement';
import type { Workspace } from '../api/gen/Workspace';
import { looksLikeKey } from '../tickets/model';
import { useTicketSearch } from '../tickets/queries';
import { TicketsList } from '../tickets/TicketsList';
import { useBacklog } from '../tickets/useBacklog';
import { UpdateBanner } from '../host/UpdateBanner';
import { useReloadOnNewBuild } from '../host/useReloadOnNewBuild';
import { KeyHelp } from '../workspaces/KeyHelp';
import { usePrefixKeys, type KeyAction } from '../workspaces/keys';
import { findTab, nextNeedingYou, type Item } from '../workspaces/model';
import { TicketTabsContext } from '../workspaces/ticketTabs';
import { NewTabChooser } from '../workspaces/NewTabChooser';
import { Sidebar } from '../workspaces/Sidebar';
import { useWorkspaces } from '../workspaces/useWorkspaces';
import { WorkspaceBar } from '../workspaces/WorkspaceBar';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { InstanceBadge } from './InstanceBadge';
import { InboxList } from '../inbox/InboxList';
import { PhoneInboxList } from '../inbox/PhoneInboxList';
import { PersonasList } from '../personas/PersonasList';
import { PmList } from '../pm/PmList';
import { noteOf, usePm, useRefreshPm, type PmSelection } from '../pm/usePm';
import { useInbox } from '../inbox/useInbox';
import { triageTitle } from '../inbox/model';
import { useDevTitle } from './useInstance';
import { NotificationSettingsPage } from '../notifications/NotificationSettingsPage';
import { useReadAllUpdates, useUpdates } from '../notifications/queries';
import { useNotificationStream } from '../notifications/useNotificationStream';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { VIEWS, type ViewId } from './views';
import { TicketKeysContext } from '../common/ticketKeys';
import type { TicketPeekState } from '../tickets/TicketsView';
import type { TicketsMode } from '../tickets/useBacklog';
import { isDetail, parseLocation, routePath, sectionSlug, upRoute, viewRoute, type Route, type SettingsPage } from './route';
import { currentLocation, goBack, navigate, useAppLinks, useLocation } from './router';
import { PhoneDrawer } from './PhoneDrawer';
import { PhoneTopBar, TopBarActionsContext, type TopBarActions } from './PhoneTopBar';

const CHOICE: Record<NewKind, Choice> = { claude: 'Claude', claude_terminal: 'Claude (terminal)', codex: 'Codex', codex_terminal: 'Codex (terminal)', devin: 'Devin', devin_terminal: 'Devin (terminal)', shell: 'Shell' };

/** On a phone, the views whose list column is their top-level screen (PDT-112). The PM's and the Tickets' lists open
 * in a sheet instead: from ⋯ (earlier sessions) and from the title (projects and filters). */
const PHONE_LIST_SCREENS: readonly ViewId[] = ['sessions', 'inbox', 'workflows', 'personas', 'files', 'settings'];

/** The Settings page of connections (in place of a project id). */
const CONNECTIONS = 'connections';
/** The Settings page of notifications. */
const NOTIFICATIONS = 'notifications';

const go = (r: Route, opts?: { replace?: boolean }) => navigate(routePath(r), opts);

/** Brings a Settings section (named in the path) into view once the page has drawn it. */
function useScrollToSection(section: string | null, page: string) {
  useEffect(() => {
    if (!section) return;
    let tries = 0;
    let timer = 0;
    const find = () => {
      const hit = [...document.querySelectorAll('main section')].find((s) => sectionSlug(s.querySelector('h3')?.textContent ?? '') === section);
      if (hit) hit.scrollIntoView?.({ block: 'start' });
      else if (tries++ < 50) timer = window.setTimeout(find, 100);
    };
    find();
    return () => window.clearTimeout(timer);
  }, [section, page]);
}

const listHeading = 'px-3 pt-3 pb-1 text-2xs uppercase tracking-wider text-muted-foreground';

/** Keeps the browser tab's title as `(<n>) [dev] <page title>`: n things need the user (the Inbox's Needs you); [dev]
 * on a dev instance. */
function useAttentionTitle(count: number) {
  useDevTitle(count);
  useReloadOnNewBuild();
}

// The ticket screens (and the Markdown libraries they use) load the first time they're shown.
const TicketsView = lazy(() => import('../tickets/TicketsView').then((m) => ({ default: m.TicketsView })));
const NewWorkflowDialog = lazy(() => import('../workflows/NewWorkflowDialog').then((m) => ({ default: m.NewWorkflowDialog })));
const NewTicketDialog = lazy(() => import('../tickets/NewTicketDialog').then((m) => ({ default: m.NewTicketDialog })));
const TicketView = lazy(() => import('../tickets/TicketView').then((m) => ({ default: m.TicketView })));
const ChangesView = lazy(() => import('../review/ChangesView').then((m) => ({ default: m.ChangesView })));
const StartDialog = lazy(() => import('../tickets/StartDialog').then((m) => ({ default: m.StartDialog })));
const WorkflowsView = lazy(() => import('../workflows/WorkflowsView').then((m) => ({ default: m.WorkflowsView })));
const WorkspaceMergedBanner = lazy(() => import('../tickets/WorkspaceMergedBanner').then((m) => ({ default: m.WorkspaceMergedBanner })));
const InboxView = lazy(() => import('../inbox/InboxView').then((m) => ({ default: m.InboxView })));
const PhoneInboxItem = lazy(() => import('../inbox/PhoneInbox').then((m) => ({ default: m.PhoneInboxItem })));
const PersonaPage = lazy(() => import('../personas/PersonaPage').then((m) => ({ default: m.PersonaPage })));
const PmView = lazy(() => import('../pm/PmView').then((m) => ({ default: m.PmView })));

export function AppShell() {
  // What is on screen comes from the path (PDT-111): selecting things navigates, and Back/Forward and reload restore
  // them. Where a path doesn't say (a workspace's last tab, Backlog or Board), the browser's stored choice does.
  const loc = useLocation();
  const route = useMemo(() => parseLocation(loc), [loc]);
  const phone = useMediaQuery(PHONE_QUERY);
  const defaultView: ViewId = phone ? 'pm' : 'sessions';
  // While a phone's on-screen keyboard is up, the shell fits the part above it, so a composer stays in view.
  const keyboardBox = useKeyboardBox(phone);
  const view: ViewId = route.view ?? defaultView;
  // The path each view showed last, so coming back to a view shows what it did (as when this was React state).
  const lastPath = useRef<Partial<Record<ViewId, string>>>({});
  useEffect(() => {
    if (route.view) lastPath.current[route.view] = loc;
  }, [route.view, loc]);
  useAppLinks();
  // The Workflows view: the workflow or template shown (with its tab and run).
  const workflowTarget: WorkflowTarget = route.view === 'workflows' ? route.target : null;
  const setWorkflowTarget = useCallback((target: WorkflowTarget) => go({ view: 'workflows', target }), []);
  const settingsPage: SettingsPage = route.view === 'settings' ? route.page : { kind: 'index' };
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  // The New workflow dialog.
  const [creatingWorkflow, setCreatingWorkflow] = useState<{ templateId?: string } | null>(null);
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
  // Phones: the drawer (☰), and the sheet holding the PM's or the Tickets' list.
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sheetOpen, setSheetOpen] = useState(false);
  // The ⋯ entries the screen on show adds (phones).
  const [screenActions, setScreenActions] = useState<{ entries: TopBarActions } | null>(null);
  // The Inbox's selected item.
  const inboxId = route.view === 'inbox' ? route.itemId : null;
  // The Personas view's persona (`new`: the create form).
  const personaId = route.view === 'personas' ? route.personaId : null;
  const setPersonaId = useCallback((personaId: string | null) => go({ view: 'personas', personaId }), []);
  const pm = usePm();
  // The PM view's session or the notes (the current session's own id reads as the current one).
  const pmRoute: PmSelection = route.view === 'pm' ? route.pm : 'current';
  const pmSelection: PmSelection = pmRoute === pm.data?.current?.id ? 'current' : pmRoute;
  const setPmSelection = useCallback((pm: PmSelection) => go({ view: 'pm', pm }), []);
  const refreshPm = useRefreshPm();
  const confirm = useConfirm();
  const inbox = useInbox();
  const inboxItems = inbox.data?.items ?? [];
  // The Inbox picks an item by itself when none is shown (or the one shown has gone): that replaces the path. On a
  // phone `/inbox` is the list, where a tap opens an item (a new entry).
  const inboxShown = !!inboxId && inboxItems.some((i) => i.id === inboxId);
  const autoPick = phone ? inboxId !== null && !inboxShown : !inboxShown;
  const setInboxId = useCallback((itemId: string | null) => go({ view: 'inbox', itemId }, { replace: autoPick }), [autoPick]);
  // A phone's item screen moves between items in place, so Back returns to the list.
  const moveInbox = useCallback((itemId: string) => go({ view: 'inbox', itemId }, { replace: true }), []);
  const plusRef = useRef<HTMLButtonElement>(null);
  const [theme, setTheme] = useThemePref();
  const qc = useQueryClient();
  // A tab chosen in the Sessions view goes in the path; the one after a closed tab replaces it (and only there).
  const onChoose = useCallback((workspaceId: string, tabId: string | null, { replace }: { replace: boolean }) => {
    if (replace && (parseLocation(currentLocation()).view ?? defaultView) !== 'sessions') return;
    go({ view: 'sessions', workspaceId, tabId }, { replace });
  }, [defaultView]);
  const ws = useWorkspaces({
    workspaceId: route.view === 'sessions' ? route.workspaceId : null,
    tabId: route.view === 'sessions' ? route.tabId : null,
    onChoose,
  });
  // The path names the workspace and tab on screen (`/sessions` alone, or a tab that has gone, is filled in).
  const wsId = ws.workspace?.id ?? null;
  const tabId = ws.tab?.id ?? null;
  useEffect(() => {
    // On a phone `/sessions` is the workspaces screen: only a workspace's path gets its tab filled in.
    if (route.view !== 'sessions' || !wsId || (phone && !route.workspaceId)) return;
    if (route.workspaceId !== wsId || route.tabId !== tabId) go({ view: 'sessions', workspaceId: wsId, tabId }, { replace: true });
  }, [route, wsId, tabId, phone]);
  // `/` shows the default view: the PM on a phone, Sessions otherwise.
  useEffect(() => {
    if (route.view === null) go(viewRoute(defaultView), { replace: true });
  }, [route.view, defaultView]);
  // A ticket workspace whose attempt merged: its banner shows above every tab (PDT-71) while the ticket is open, or
  // Done with its worktree still there (the banner checks that, PDT-97).
  const wsTicket = ws.workspace?.ticket;
  const mergedTicketId = wsTicket && wsTicket.attemptState === 'merged' && wsTicket.stage !== 'wont_do' ? wsTicket.id : null;
  const stored = useBacklog();
  // Backlog or Board: the path says (`?view=board`); the stored choice is what the Tickets view opens with.
  const ticketsRoute = route.view === 'tickets' ? route : null;
  const mode: TicketsMode = ticketsRoute ? (ticketsRoute.board ? 'board' : 'backlog') : stored.mode;
  useEffect(() => {
    if (stored.mode !== mode) stored.setMode(mode);
  }, [stored.mode, mode, stored.setMode]);
  const peekRef = ticketsRoute?.ticket ?? null;
  const backlog = {
    ...stored,
    mode,
    setMode: (m: TicketsMode) => go({ view: 'tickets', board: m === 'board', ticket: peekRef }),
  };
  const peek: TicketPeekState = useMemo(() => ({
    ticket: peekRef,
    // On a phone the ticket is a detail screen, so closing it is ← (Back), not a new entry.
    onChange: (ticket, opts) => (phone && ticket === null && peekRef !== null
      ? goBack(routePath({ view: 'tickets', board: mode === 'board', ticket: null }))
      : go({ view: 'tickets', board: mode === 'board', ticket }, opts)),
  }), [peekRef, mode, phone]);
  /** Shows a view as it was last (a view's own path the first time). */
  const setView = useCallback((v: ViewId) => {
    if ((parseLocation(currentLocation()).view ?? defaultView) === v) return;
    if (v === 'sessions') navigate('/sessions');
    else if (v === 'tickets') go({ view: 'tickets', board: stored.mode === 'board', ticket: null });
    else navigate(lastPath.current[v] ?? routePath(viewRoute(v)));
  }, [defaultView, stored.mode]);
  // The view changes once the tab is there: one that can't open (its ticket or session is gone) leaves the user where
  // they were with the error, not on whatever tab was current (PDT-93).
  const openTicket = useCallback((t: { id: string; projectId: string }) => {
    void runAction(async () => {
      if (await ws.focusItem({ kind: 'ticket', id: t.id }, t.projectId)) setView('sessions');
    });
  }, [ws.focusItem, setView]);
  const openTicketByKey = useCallback((key: string) => {
    void runAction(async () => {
      const hit = (await api.listTickets({ q: key, limit: 1 })).items[0];
      if (hit) openTicket(hit);
    });
  }, [openTicket]);
  const pmSessions = pm.data?.sessions;
  const openSession = useCallback((s: { id: string; projectId: string }) => {
    // A PM session has no tab: the PM view shows it.
    if (pmSessions?.some((x) => x.session.id === s.id)) {
      setPmSelection(pm.data?.current?.id === s.id ? 'current' : s.id);
      return;
    }
    void runAction(async () => {
      if (await ws.focusItem({ kind: 'session', id: s.id }, s.projectId)) setView('sessions');
    });
  }, [ws.focusItem, pmSessions, pm.data?.current?.id, setPmSelection, setView]);
  // A new PM session ends the current one (it writes no journal), so it asks first.
  const newPmSession = useCallback(() => {
    setView('pm');
    setSheetOpen(false);
    if (!pm.data?.current) {
      setPmSelection('current');
      return;
    }
    void runAction(async () => {
      const { ok } = await confirm({
        title: 'Start a new PM session?',
        body: 'The current one ends and stays readable. The new one starts from a fresh briefing and the notes.',
        action: 'New session',
      });
      if (!ok) return;
      await api.startPm();
      await refreshPm();
      setPmSelection('current');
    });
  }, [pm.data?.current, confirm, refreshPm, setView, setPmSelection]);
  // After a ticket's tab, in the workspace that has it (its Start workspace).
  const placementNear = useCallback((ticketId: string): TabPlacement | undefined => {
    const hit = findTab(ws.workspaces, { kind: 'ticket', id: ticketId });
    return hit ? { workspaceId: hit.workspace.id, position: hit.tab.position + 1 } : undefined;
  }, [ws.workspaces]);
  // Opening a tab shows Sessions: the tab's selection navigates there.
  const openChanges = useCallback((w: { id: string; projectId: string }, opts?: OpenChangesOptions) => {
    const item = { kind: 'changes', id: w.id } as const;
    const near = opts?.ticketId ? placementNear(opts.ticketId) : undefined;
    const here = opts?.here ? ws.workspace : near ? ws.workspaces.find((x) => x.id === near.workspaceId) : undefined;
    const existing = here?.tabs.find((t) => t.worktreeId === w.id);
    const position = near?.position ?? (ws.tab ? ws.tab.position + 1 : here?.tabs.length);
    if (here && existing) ws.selectTab(existing);
    else if (here) void runAction(() => ws.openIn(item, here.id, position));
    else void runAction(() => ws.focusItem(item, w.projectId));
  }, [ws.focusItem, ws.openIn, ws.selectTab, ws.workspace, ws.workspaces, ws.tab, placementNear]);
  const openRun = useCallback((r: { id: string; projectId: string }) => {
    void runAction(async () => {
      await ws.refresh();
      await ws.focusItem({ kind: 'run', id: r.id }, r.projectId);
    });
  }, [ws.focusItem, ws.refresh]);
  const openNear = useCallback((item: Item, projectId: string, ticketId: string) => {
    const existing = findTab(ws.workspaces, item);
    const near = placementNear(ticketId);
    if (existing) ws.selectTab(existing.tab);
    else if (near) void runAction(() => ws.openIn(item, near.workspaceId, near.position));
    else void runAction(() => ws.focusItem(item, projectId));
  }, [ws.workspaces, ws.selectTab, ws.openIn, ws.focusItem, placementNear]);
  const ticketTabs = useMemo(() => ({ placementNear, openRun, openNear }), [placementNear, openRun, openNear]);
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
    const projectId = r.ticket.projectId;
    void qc.invalidateQueries({ queryKey: ['sessions', projectId] });
    void qc.invalidateQueries({ queryKey: ['worktrees', projectId] });
    void runAction(async () => {
      await ws.refresh();
      // Started without a session: the ticket's tab.
      await ws.focusItem(r.session ? { kind: 'session', id: r.session.id } : { kind: 'ticket', id: r.ticket.id }, projectId);
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
      go({ view: 'tickets', board: a.type === 'board', ticket: null });
      return;
    }
    if (a.type === 'inbox') {
      setView('inbox');
      return;
    }
    if (a.type === 'pm') {
      setView('pm');
      return;
    }
    setView('sessions');
    switch (a.type) {
      case 'newTab': setChooserOpen(true); break;
      case 'switcher':
        // Focus the current workspace in the sidebar; Up/Down and Enter pick another. On a phone, open the sheet first.
        if (phone) setDrawerOpen(true);
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

  useAttentionTitle(inbox.data?.total ?? 0);

  const label = VIEWS.find((v) => v.id === view)!.label;
  const current = ws.currentInfo;
  const settingsProject = settingsPage.kind === 'project' ? ws.projects.find((p) => p.key === settingsPage.key) ?? null : null;
  // The Settings list's current row: a project's id, connections, or null (this host).
  const settingsProjectId = settingsProject?.id
    ?? (settingsPage.kind === 'connections' ? CONNECTIONS : settingsPage.kind === 'notifications' ? NOTIFICATIONS : null);
  const section = settingsPage.kind === 'host' || settingsPage.kind === 'project' ? settingsPage.section : null;
  useScrollToSection(view === 'settings' ? section : null, settingsProjectId ?? '');
  const currentProject = ws.projects.find((p) => p.id === current?.projectId) ?? null;
  const worktrees = useQuery({
    queryKey: ['worktrees', currentProject?.id ?? ''],
    queryFn: () => api.listWorktrees(currentProject!.id),
    enabled: !!currentProject && !!current?.worktreeId,
  });
  const worktreeBranch = current?.worktreeId ? worktrees.data?.find((w) => w.id === current.worktreeId)?.branch : undefined;
  const branch = current?.run?.branch ?? worktreeBranch ?? null;
  const branchLink = currentProject?.githubRepo && branch ? branchUrl(currentProject.githubRepo, branch) : null;
  // The Files view: the file in the path, in the checkout the path names (a project key and worktree) or the
  // current tab's.
  const filesRoute = route.view === 'files' ? route : null;
  const filesProject = filesRoute?.project ? ws.projects.find((p) => p.key === filesRoute.project) : undefined;
  const folder: FolderViewState = useFolderView(ws.projects, current ? { projectId: current.projectId, worktreeId: current.worktreeId } : null, {
    picked: filesProject ? { projectId: filesProject.id, worktreeId: filesRoute?.worktree ?? null } : null,
    file: filesRoute?.path ?? null,
    onChange: (picked, file) => {
      // A file's path names its checkout, so a reload shows the same file even if the current tab changes.
      const at = picked ?? (file ? folder.place : null);
      go({ view: 'files', path: file, project: at ? ws.projects.find((p) => p.id === at.projectId)?.key ?? null : null, worktree: at?.worktreeId ?? null });
    },
  });
  const bottomProject = ws.projects.find((p) => p.id === (current?.projectId ?? ws.workspace?.projectId)) ?? ws.projects[0] ?? null;

  // Choosing a file closes the phone sheet.
  useEffect(() => {
    if (folder.file) setSheetOpen(false);
  }, [folder.file]);

  /** Settings for a project (by id), connections, notifications, or this host (null). */
  const showProjectSettings = (id: string | null) => {
    const key = ws.projects.find((p) => p.id === id)?.key;
    go({
      view: 'settings',
      page: id === CONNECTIONS ? { kind: 'connections' } : id === NOTIFICATIONS ? { kind: 'notifications' }
        : key ? { kind: 'project', key, section: null } : { kind: 'host', section: null },
    });
    setSheetOpen(false);
  };

  // Notifications (PDT-108): the Inbox's Updates, the stream's toasts and desktop notifications, and where a click goes.
  const updates = useUpdates();
  const readAllUpdates = useReadAllUpdates();
  const unreadUpdates = (updates.data?.unreadUpdates ?? 0) > 0;
  const openTarget = useCallback((t: NotificationTarget) => {
    switch (t.kind) {
      case 'session': openSession({ id: t.id, projectId: t.projectId }); break;
      case 'ticket': openTicket({ id: t.id, projectId: t.projectId }); break;
      case 'ticket_key': openTicketByKey(t.key); break;
      case 'inbox': go({ view: 'inbox', itemId: t.id }); break;
      case 'workflow': setWorkflowTarget({ kind: 'workflow', id: t.id }); break;
      case 'url': window.open(t.url, '_blank', 'noopener'); break;
      case 'none': break;
    }
  }, [openSession, openTicket, openTicketByKey, setWorkflowTarget]);
  const onScreenSession = view === 'sessions' && current?.item.kind === 'session' ? current.item.id : null;
  const onScreenInbox = view === 'inbox' ? inboxId : null;
  const onScreen = useMemo(() => ({ session: onScreenSession, inbox: onScreenInbox }), [onScreenSession, onScreenInbox]);
  useNotificationStream(openTarget, onScreen);

  // The palette's items, built only while it is open (the screen re-renders every 2 s).
  const place: Place | null = current
    ? { projectId: current.projectId, worktreeId: current.worktreeId }
    : ws.workspace?.projectId ? { projectId: ws.workspace.projectId, worktreeId: null } : null;
  // New tickets go in the backlog's project in the Tickets view, else the current workspace's project.
  const ticketProjectId = view === 'tickets' ? backlog.projectId : place?.projectId ?? backlog.projectId;
  const handlers: PaletteHandlers = {
    selectWorkspace: (id) => ws.selectWorkspace(id),
    selectTab: (t) => ws.selectTab(t),
    focusItem: (item, projectId) => void runAction(() => ws.focusItem(item, projectId)),
    projectSettings: (id) => showProjectSettings(id),
    hostSettings: () => showProjectSettings(null),
    newSession: (kind, at) => {
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
    renameTab: (t) => { ws.selectTab(t); setRenamingTabId(t.id); },
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
    showWorkflows: () => setView('workflows'),
    showInbox: () => setView('inbox'),
    showPersonas: () => setView('personas'),
    showPm: () => setView('pm'),
    newTicket: () => setNewTicket({ projectId: ticketProjectId, title: '' }),
    openTicket,
    startTicket: (t, here) => startTicket(t, { here }),
    startNextPart: (t) => void runAction(() => startNextPart(startTicket, t)),
  };
  const agents = useAgentAvailability();
  const defaults = useAgentDefaults();
  const items = paletteOpen
    ? paletteItems({
        workspaces: ws.workspaces, infos: [...ws.allInfos, ...ws.ticketInfos], projects: ws.projects, tab: ws.tab, place, theme, handlers,
        tickets: looksLikeKey(keyQuery) ? ticketHits.data?.items ?? [] : [],
        ticketProjectId: view === 'tickets' ? backlog.projectId : null,
        agents: agents.of,
        defaults,
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
  ) : view === 'inbox' ? (
    <InboxList items={inboxItems} updates={updates.data?.items ?? []} selectedId={inboxId} loading={inbox.isLoading}
      onSelect={(id) => { setInboxId(id); setSheetOpen(false); }}
      onReadAll={() => readAllUpdates.mutate()} onSettings={() => showProjectSettings(NOTIFICATIONS)} />
  ) : view === 'pm' ? (
    <PmList selection={pmSelection} onSelect={(sel) => { setPmSelection(sel); setSheetOpen(false); }} onNew={newPmSession} />
  ) : view === 'personas' ? (
    <PersonasList selectedId={personaId} onSelect={(id) => { setPersonaId(id); setSheetOpen(false); }} />
  ) : view === 'tickets' ? (
    <TicketsList backlog={backlog} projects={ws.projects} onChoose={() => setSheetOpen(false)} />
  ) : view === 'workflows' ? (
    <WorkflowsList projects={ws.projects} current={workflowTarget}
      onOpen={(t) => { setWorkflowTarget(t); setSheetOpen(false); }}
      onNew={() => { setSheetOpen(false); setCreatingWorkflow({}); }} />
  ) : view === 'files' ? (
    <>
      <h2 className={listHeading}>{label}</h2>
      <FolderTree state={folder} />
    </>
  ) : (
    <nav aria-label="Settings sections" className="flex flex-col gap-0.5 py-1">
      <h2 className={listHeading}>{label}</h2>
      <List aria-label="Host" onAction={(id) => showProjectSettings(id === CONNECTIONS || id === NOTIFICATIONS ? id : null)}>
        <ListRow id="host" title="This host" current={!phone && settingsProjectId === null} />
        <ListRow id={NOTIFICATIONS} title="Notifications" current={settingsProjectId === NOTIFICATIONS} />
        <ListRow id={CONNECTIONS} title="Connections" current={settingsProjectId === CONNECTIONS} />
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
  // The phone's screens (PDT-112): a view's top-level screen (its list, for most views) or a detail screen with ←.
  const detail = isDetail(route);
  const phoneList = phone && !detail && PHONE_LIST_SCREENS.includes(view);
  const pickerSheet = phone && view === 'tickets';
  const ticketsProject = ws.projects.find((p) => p.id === backlog.projectId);
  const phoneTitle: ReactNode = view === 'tickets' && !detail ? (
    <Button variant="ghost" size="sm" aria-label="Projects and filters" aria-haspopup="dialog" onClick={() => setSheetOpen(true)} className="min-w-0 font-medium">
      <span className="truncate">{ticketsProject ? `${label} · ${ticketsProject.key}` : label}</span>
      <ChevronDown aria-hidden className="size-4 text-muted-foreground" />
    </Button>
  ) : view === 'tickets' && detail ? peekRef ?? label
  : view === 'inbox' && detail && inboxId ? (triageTitle(inboxItems, inboxId) ?? label)
  : view === 'sessions' && detail ? (ws.workspace?.ticket ? `${ws.workspace.ticket.key} ${ws.workspace.ticket.title}` : ws.workspace?.name ?? label)
  : view === 'pm' && detail ? (pmSelection === 'notes' ? 'Notes' : noteOf(pmSelection) ? 'Note' : pmSelection === 'earlier' ? 'Earlier sessions' : 'Earlier PM session')
  : label;
  const shellActions = (): MenuEntry[] =>
    view === 'pm' ? [
      { label: 'New session…', onSelect: newPmSession },
      { label: 'Notes', onSelect: () => setPmSelection('notes') },
      { label: 'Earlier sessions', onSelect: () => setPmSelection('earlier') },
    ] : view === 'sessions' && detail ? [
      { label: 'New tab', onSelect: () => setChooserOpen(true) },
      { label: bottomOpen ? 'Hide terminal panel' : 'Terminal panel', onSelect: () => setBottomOpen((o) => !o) },
    ] : [];
  const hasShellActions = view === 'pm' || (view === 'sessions' && detail);
  const phoneActions: TopBarActions | null = hasShellActions || screenActions
    ? () => [...shellActions(), ...(screenActions?.entries() ?? [])]
    : null;
  const projectKeyList = ws.projects.map((p) => p.key).join(' ');
  const projectKeys = useMemo(() => (projectKeyList ? projectKeyList.split(' ') : []), [projectKeyList]);

  return (
    <TicketKeysContext.Provider value={projectKeys}>
    <OpenTicketContext.Provider value={openTicket}>
    <OpenSessionContext.Provider value={openSession}>
    <OpenChangesContext.Provider value={openChanges}>
    <TicketTabsContext.Provider value={ticketTabs}>
    <StartTicketContext.Provider value={startTicket}>
    <TopBarActionsContext.Provider value={setScreenActions}>
    <div
      className="flex h-full flex-col overflow-hidden bg-background text-foreground md:flex-row"
      style={keyboardBox ? { position: 'fixed', left: 0, right: 0, top: keyboardBox.top, height: keyboardBox.height } : undefined}
    >
      {phone ? (
        <>
          <PhoneTopBar
            detail={detail}
            onMenu={() => setDrawerOpen(true)}
            menuOpen={drawerOpen}
            onBack={() => goBack(routePath(upRoute(route)))}
            title={phoneTitle}
            needsYou={inbox.data?.total ?? 0}
            onInbox={() => go(viewRoute('inbox'))}
            onSearch={() => setPaletteOpen(true)}
            actions={phoneActions}
          />
          <PhoneDrawer
            open={drawerOpen}
            onOpenChange={setDrawerOpen}
            view={view}
            onView={setView}
            counts={{ inbox: inbox.data?.total ?? 0 }}
            dots={{ inbox: unreadUpdates }}
            ws={ws}
            projectId={stored.projectId}
            onProject={stored.setProjectId}
            onRenameTab={setRenamingTabId}
          />
          {pickerSheet && (
            <Sheet side="left" title="Projects and filters" hideTitle open={sheetOpen} onOpenChange={setSheetOpen} className="w-72 max-w-[85vw]">
              <div className="-mx-2">{list}</div>
            </Sheet>
          )}
        </>
      ) : (
        <>
          <IconRail
            active={view}
            onSelect={setView}
            top={<InstanceBadge stacked />}
            counts={{ inbox: inbox.data?.total ?? 0 }}
            dots={{ inbox: unreadUpdates }}
          />
          <aside aria-label={`${label} list`} className="flex min-h-0 w-72 flex-col border-r border-border">
            <div className="flex-1 overflow-auto px-1">{list}</div>
          </aside>
        </>
      )}
      <main className="relative flex-1 flex flex-col min-w-0 min-h-0">
        <UpdateBanner />
        {view === 'sessions' && !phoneList ? (
          <WorkspaceBar
            ws={ws}
            renamingTabId={renamingTabId}
            setRenamingTabId={setRenamingTabId}
            onNewTab={() => setChooserOpen(true)}
            plusRef={plusRef}
            hideName={phone}
            // On a phone the terminal panel is under ⋯.
            trailing={phone ? undefined : (
              <IconButton label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
                <PanelBottom aria-hidden />
              </IconButton>
            )}
          />
        ) : null}
        {view === 'sessions' && !phoneList && mergedTicketId && (
          <Suspense fallback={null}><WorkspaceMergedBanner key={mergedTicketId} ticketId={mergedTicketId} /></Suspense>
        )}
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
                go({ view: 'settings', page: { kind: 'project', key: p.key, section: null } });
                return;
              }
              // Show the new project's workspace once the list includes it.
              void runAction(async () => {
                await ws.refresh();
                const home = qc.getQueryData<Workspace[]>(['workspaces'])?.find((w) => w.projectId === p.id);
                if (home) ws.selectWorkspace(home.id);
              });
            }}
          />
        )}
        {!(bottomOpen && bottomMax) && (
          <div className="flex-1 flex flex-col min-h-0">
            {phoneList && view === 'inbox' ? (
              <PhoneInboxList items={inboxItems} updates={updates.data?.items ?? []} loading={inbox.isLoading}
                onOpen={(id) => go({ view: 'inbox', itemId: id })}
                onReadAll={() => readAllUpdates.mutate()} onSettings={() => showProjectSettings(NOTIFICATIONS)} />
            ) : phoneList ? (
              <div className="flex-1 overflow-auto px-1">
                {list}
                {view === 'workflows' && creatingWorkflow && (
                  <Suspense fallback={null}><NewWorkflowDialog projects={ws.projects} templateId={creatingWorkflow.templateId}
                    onClose={() => setCreatingWorkflow(null)}
                    onCreated={(id) => { setCreatingWorkflow(null); setWorkflowTarget({ kind: 'workflow', id }); }} /></Suspense>
                )}
              </div>
            ) : view === 'inbox' && phone && inboxId ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><PhoneInboxItem items={inboxItems} updates={updates.data?.items ?? []}
                itemId={inboxId} loading={inbox.isLoading} onMove={moveInbox} onList={() => goBack(routePath(viewRoute('inbox')))} onOpenTarget={openTarget} /></Suspense>
            ) : view === 'inbox' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><InboxView items={inboxItems} updates={updates.data?.items ?? []} selectedId={inboxId} onSelect={setInboxId} onOpenTarget={openTarget} /></Suspense>
            ) : view === 'pm' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><PmView selection={pmSelection} onSelect={setPmSelection} onNew={newPmSession} onBack={() => goBack(routePath(upRoute(route)))} /></Suspense>
            ) : view === 'personas' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><PersonaPage key={personaId ?? 'none'} personaId={personaId} onSelect={setPersonaId} /></Suspense>
            ) : view === 'tickets' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><TicketsView backlog={backlog} projects={ws.projects} peek={peek} onNewTicket={(projectId, title) => setNewTicket({ projectId, title: title ?? '' })} /></Suspense>
            ) : view === 'workflows' ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><WorkflowsView target={workflowTarget} onOpen={setWorkflowTarget} projects={ws.projects} creating={creatingWorkflow} onCreate={setCreatingWorkflow} /></Suspense>
            ) : view === 'files' ? (
              <FileViewer state={folder} />
            ) : view === 'settings' ? (
              settingsPage.kind === 'project' ? (
                settingsProject ? <ProjectPage key={settingsProject.id} project={settingsProject} onRemoved={() => showProjectSettings(null)} />
                  : ws.projectsLoaded ? <EmptyState>No project has the key {settingsPage.key}.</EmptyState> : <Skeleton lines={8} className="p-4" />
              ) : settingsPage.kind === 'connections' ? <ConnectionsPage />
                : settingsPage.kind === 'notifications' ? <NotificationSettingsPage /> : <HostSettingsPage />
            ) : ws.tab?.ticketId ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><TicketView key={ws.tab.ticketId} ticketId={ws.tab.ticketId} mergedBanner={ws.tab.ticketId !== mergedTicketId} onDeleted={() => void ws.refresh()} /></Suspense>
            ) : ws.tab?.worktreeId ? (
              <Suspense fallback={<Skeleton lines={8} className="p-4" />}><ChangesView key={ws.tab.worktreeId} worktreeId={ws.tab.worktreeId} /></Suspense>
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
    </TopBarActionsContext.Provider>
    </StartTicketContext.Provider>
    </TicketTabsContext.Provider>
    </OpenChangesContext.Provider>
    </OpenSessionContext.Provider>
    </OpenTicketContext.Provider>
    </TicketKeysContext.Provider>
  );
}
