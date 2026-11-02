import { Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, FileText, NotebookPen, PanelBottom } from 'lucide-react';
import { api } from '../api/client';
import { branchUrl } from '../projects/forgeLinks';
import { Button, EmptyState, IconButton, Link, Sheet, Skeleton, useConfirm, useThemePref, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import type { Place } from '../common/PlacePicker';
import { runAction } from '../common/useAction';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { isAcp, useAcpAgents, type AcpKind } from '../common/agents';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { useKeyboardBox } from '../common/viewport';
import { CommandPalette } from '../palette/CommandPalette';
import { paletteItems, type NewKind, type PaletteHandlers } from '../palette/paletteItems';
import { useMetaK } from '../palette/useMetaK';
import { AddProjectDialog } from '../projects/AddProjectDialog';
import type { AddProjectMode } from '../projects/AddProjectForm';
import { SetupPage } from '../setup/SetupPage';
import { choiceFor, newSession, type Choice } from '../sessions/newSession';
import { ProjectPage } from '../projects/ProjectPage';
import { FileViewer, FolderList, FolderTree, useFolderView, type FolderViewState } from '../files/FolderView';
import { RunView } from '../runs/RunView';
import { SessionView } from '../sessions/SessionView';
import { HostSettingsPage } from '../host/HostSettingsPage';
import { sectionLabel } from '../common/SettingsScreens';
import { HOST_SECTION_LABELS, PROJECT_SECTION_LABELS } from '../common/settingsSections';
import { SettingsNav } from './SettingsNav';
import { settingsPageScreen, settingsPageTitle, type SettingsPageContext } from './settingsPages';
import { pluginsKey } from '../plugins/model';
import { WorkflowsList, type WorkflowTarget } from '../workflows/WorkflowsList';
import { OpenSessionContext, OpenTicketContext, StartTicketContext, startNextPart } from '../tickets/actions';
import { invalidateTickets } from '../tickets/queries';
import { OpenChangesContext, type OpenChangesOptions } from '../review/open';
import type { StartResult } from '../api/gen/StartResult';
import type { TabPlacement } from '../api/gen/TabPlacement';
import type { Workspace } from '../api/gen/Workspace';
import { looksLikeKey } from '../tickets/model';
import { ticketByKey, useProjectForge, useTicketSearch } from '../tickets/queries';
import { TicketsList } from '../tickets/TicketsList';
import { useBacklog } from '../tickets/useBacklog';
import { UpdateBanner } from '../host/UpdateBanner';
import { useReloadOnNewBuild } from '../host/useReloadOnNewBuild';
import { KeyHelp } from '../workspaces/KeyHelp';
import { BugReportDialog } from '../host/BugReportDialog';
import { usePrefixKeys, type KeyAction } from '../workspaces/keys';
import { findTab, nextNeedingYou, type Item } from '../workspaces/model';
import { TicketTabsContext } from '../workspaces/ticketTabs';
import { NewTabChooser } from '../workspaces/NewTabChooser';
import { TabOverview } from '../workspaces/TabOverview';
import { InlineRename } from '../common/InlineRename';
import { Sidebar } from '../workspaces/Sidebar';
import { useWorkspaces } from '../workspaces/useWorkspaces';
import { WorkspaceBar, workspaceTabId } from '../workspaces/WorkspaceBar';
import { BottomPanel } from './BottomPanel';
import { RightPanel, RightPanelContent, useRightPanelKey, useRightPanelOpen } from './RightPanel';
import type { TodoFocus } from '../todos/TodoList';
import { IconRail } from './IconRail';
import { InstanceBadge } from './InstanceBadge';
import { InboxList } from '../inbox/InboxList';
import { PhoneInboxList } from '../inbox/PhoneInboxList';
import { PersonasList } from '../personas/PersonasList';
import { PmList } from '../pm/PmList';
import { pmUnread, shownConversation, usePm, type PmSelection } from '../pm/usePm';
import { NotesList } from '../notes/NotesList';
import { useNotesFilters } from '../notes/useNotes';
import { DocumentsList, useDocumentsFilters } from '../documents/DocumentsList';
import { ImportDocumentDialog, NewDocumentDialog } from '../documents/DocumentDialogs';
import type { Whose } from '../notes/model';
import { useInbox } from '../inbox/useInbox';
import { triageTitle } from '../inbox/model';
import { ErrorBoundary } from '../common/ErrorBoundary';
import { useDevTitle } from './useInstance';
import { useReadAllUpdates, useClearUpdate, useClearAllUpdates, useUpdates } from '../notifications/queries';
import { useNotificationStream } from '../notifications/useNotificationStream';
import { useAttention, useFaviconBadge, type Attention } from '../notifications/attention';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { VIEWS, type ViewId } from './views';
import { TicketKeysContext } from '../common/ticketKeys';
import type { TicketPeekState } from '../tickets/TicketsView';
import type { TicketsMode } from '../tickets/useBacklog';
import { readViewState, tabScope, ViewScopeContext, writeViewState } from '../common/viewState';
import { isDetail, isOldPath, parseLocation, routePath, upRoute, viewRoute, type Route, type SettingsPage } from './route';
import { currentLocation, goBack, navigate, useAppLinks, useLocation } from './router';
import { PhoneDrawer } from './PhoneDrawer';
import { PhoneTopBar, TopBarActionsContext, TopBarEditContext, type TopBarActions, type TopBarEdit } from './PhoneTopBar';
import {
  ChangesView, CreateIssueDialog, DocumentPage, InboxView, NewTicketDialog, NewWorkflowDialog, NoteScreen, PersonaPage, PhoneInboxItem, PmView,
  ScreenSuspense, StartDialog, TicketView, TicketsView, WorkflowsView, WorkspaceMergedBanner,
} from './screens';

const CHOICE: Record<Exclude<NewKind, AcpKind>, Choice> = { claude: 'Claude', claude_terminal: 'Claude (terminal)', codex: 'Codex', codex_terminal: 'Codex (terminal)', devin: 'Devin', devin_terminal: 'Devin (terminal)', shell: 'Shell' };

/** On a phone, the views whose list column is their top-level screen (PDT-112). The PM's and the Tickets' lists open
 * in a sheet instead: from ⋯ (earlier sessions) and from the title (projects and filters). */
const PHONE_LIST_SCREENS: readonly ViewId[] = ['pm', 'sessions', 'inbox', 'notes', 'documents', 'workflows', 'personas', 'files', 'settings'];

/** The Settings page of plugins (PDT-228). */
const PLUGINS = 'plugins';
/** The Settings page of all connections (in place of a project id). */
const CONNECTIONS = 'connections';
/** The Settings page of notifications. */
const NOTIFICATIONS = 'notifications';
/** The Settings page of secrets (PDT-158). */
const SECRETS = 'secrets';

const go = (r: Route, opts?: { replace?: boolean }) => navigate(routePath(r), opts);

const listHeading = 'px-3 pt-3 pb-1 text-2xs uppercase tracking-wider text-muted-foreground';

/** The page title's `(n)` (the Inbox's items) and the favicon's dot (the most important thing waiting, PDT-207), and
 * reloading on a new build. */
function useAttentionTitle(attention: Attention) {
  useDevTitle(attention.count);
  useFaviconBadge(attention.level);
  useReloadOnNewBuild();
}

/** The view-state key of the path each view showed last. */
const LAST_PATHS = 'app|lastPaths';

export function AppShell() {
  // What is on screen comes from the path (PDT-111): selecting things navigates, and Back/Forward and reload restore
  // them. Where a path doesn't say (a workspace's last tab, Backlog or Board), the browser's stored choice does.
  const loc = useLocation();
  const route = useMemo(() => parseLocation(loc), [loc]);
  const phone = useMediaQuery(PHONE_QUERY);
  const defaultView: ViewId = phone ? 'pm' : 'sessions';
  // Get started (PDT-230) shows in the Sessions view's place: its sidebar lists the projects as they are added.
  const setupShown = route.view === 'setup';
  // Whether the app opens on Get started (a new install, until it is set up or dismissed).
  const setup = useQuery({ queryKey: ['host-setup'], queryFn: api.hostSetup, retry: false, staleTime: Infinity });
  // While a phone's on-screen keyboard is up, the shell fits the part above it, so a composer stays in view.
  const keyboardBox = useKeyboardBox(phone);
  const view: ViewId = route.view === 'setup' ? 'sessions' : route.view ?? defaultView;
  // The path each view showed last, so coming back to a view shows what it did (Settings' page and section, a
  // document); kept with the views' state, so it lasts the browser tab's session.
  const lastPath = useRef<Partial<Record<ViewId, string>>>(readViewState(LAST_PATHS) ?? {});
  useEffect(() => {
    if (route.view && route.view !== 'setup') {
      lastPath.current[route.view] = loc;
      writeViewState(LAST_PATHS, { ...lastPath.current });
    }
  }, [route.view, loc]);
  useAppLinks();
  // An old path (the Integrations page, This host's GitHub section) shows its new page under the new path.
  useEffect(() => {
    if (isOldPath(loc.split('?')[0])) navigate(routePath(route), { replace: true });
  }, [loc, route]);
  // The Workflows view: the workflow or template shown (with its tab and run).
  const workflowTarget: WorkflowTarget = route.view === 'workflows' ? route.target : null;
  const setWorkflowTarget = useCallback((target: WorkflowTarget) => go({ view: 'workflows', target }), []);
  const settingsPage: SettingsPage = route.view === 'settings' ? route.page : { kind: 'index' };
  const openPlugin = useCallback((id: string) => go({ view: 'settings', page: { kind: 'plugins', id } }), []);
  // A plugin page's name, for a phone's top bar.
  const shownPlugins = useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, enabled: settingsPage.kind === 'plugins' && settingsPage.id !== null });
  const pluginTitle = (id: string) => shownPlugins.data?.find((p) => p.id === id)?.label ?? 'Plugin';
  const settingsContext: SettingsPageContext = { phone, openPlugin, pluginTitle };
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  // The New workflow dialog.
  const [creatingWorkflow, setCreatingWorkflow] = useState<{ templateId?: string } | null>(null);
  const [chooserOpen, setChooserOpen] = useState(false);
  const [renamingTabId, setRenamingTabId] = useState<string | null>(null);
  // Phones: the tab overview (⧉), the workspace being renamed in the top bar, and the New workspace field on
  // `/sessions` (from ⋯).
  const [overviewOpen, setOverviewOpen] = useState(false);
  const [renamingWorkspace, setRenamingWorkspace] = useState(false);
  const [addingWorkspace, setAddingWorkspace] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [reportingBug, setReportingBug] = useState(false);
  const [rightOpen, setRightOpen] = useRightPanelOpen();
  const [todosSheetOpen, setTodosSheetOpen] = useState(false);
  // A todo is open in the phone's sheet: it takes the whole screen (PDT-227).
  const [todoOpenInSheet, setTodoOpenInSheet] = useState(false);
  // Bumped when Ctrl+] t opens the todo list: its add box takes the focus.
  const [todoAddSeq, setTodoAddSeq] = useState(0);
  // A todo a due reminder opened (PDT-140).
  const [todoFocus, setTodoFocus] = useState<TodoFocus | null>(null);
  const clearTodoFocus = useCallback(() => setTodoFocus(null), []);
  const [addingProject, setAddingProject] = useState(false);
  // The way of adding a project the dialog opens on (Get started's Clone from GitHub opens on Clone URL).
  const [addMode, setAddMode] = useState<AddProjectMode>('path');
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  // The New ticket form: its project and the title typed so far.
  const [newTicket, setNewTicket] = useState<{ projectId: string | null; title: string } | null>(null);
  const [newIssue, setNewIssue] = useState<{ projectId: string; repo: string } | null>(null);
  // The Start dialog: the ticket, and where its tabs open (undefined: the end of the project workspace).
  const [starting, setStarting] = useState<{ ticketId: string; placement?: TabPlacement } | null>(null);
  // Phones: the drawer (☰), and the sheet holding the PM's or the Tickets' list.
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sheetOpen, setSheetOpen] = useState(false);
  // The ⋯ entries the screen on show adds (phones).
  const [screenActions, setScreenActions] = useState<{ entries: TopBarActions } | null>(null);
  // An edit on the screen on show: Cancel and Save in the top bar (phones).
  const [screenEdit, setScreenEdit] = useState<TopBarEdit | null>(null);
  // The Inbox's selected item.
  const inboxId = route.view === 'inbox' ? route.itemId : null;
  // The Personas view's persona (`new`: the create form).
  const personaId = route.view === 'personas' ? route.personaId : null;
  const setPersonaId = useCallback((personaId: string | null) => go({ view: 'personas', personaId }), []);
  const pm = usePm();
  // The PM view's conversation (PDT-149): `current` is the newest open one (the list, on a phone).
  const pmSelection: PmSelection = route.view === 'pm' ? route.pm : 'current';
  const setPmSelection = useCallback((pm: PmSelection) => go({ view: 'pm', pm }), []);
  // The Notes view (PDT-143): the note on show (`new`: one not created yet) and the list's side, scope and search.
  const notes = useNotesFilters();
  const setNotesWhose = notes.setWhose;
  const noteId = route.view === 'notes' ? route.note : null;
  useEffect(() => {
    if (route.view === 'notes' && route.note === null) setNotesWhose(route.memory ? 'note' : 'user');
  }, [route, setNotesWhose]);
  // The notes' old paths (`/pm/notes…`) show the new ones.
  useEffect(() => {
    if (route.view === 'notes' && loc.startsWith('/pm/')) go(route, { replace: true });
  }, [route, loc]);
  const notesList = (whose: Whose): Route => ({ view: 'notes', memory: whose === 'note', note: null });
  const openNote = useCallback((id: string) => go({ view: 'notes', memory: false, note: id }), []);
  const newNote = () => go({ view: 'notes', memory: notes.whose === 'note', note: 'new' });
  // The Documents view (PDT-314): the document on show, the list's filters, and New or Import open.
  const documents = useDocumentsFilters();
  const docId = route.view === 'documents' ? route.doc : null;
  const openDoc = useCallback((id: string) => go({ view: 'documents', doc: id }), []);
  const [docDialog, setDocDialog] = useState<'new' | 'import' | null>(null);
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
  // `/` shows the default view: the PM on a phone, Sessions otherwise; Get started instead while it opens on launch
  // (once it has said whether it does).
  const opensOnSetup = setup.isPending ? null : setup.data?.opensOnLaunch ?? false;
  useEffect(() => {
    if (route.view !== null || opensOnSetup === null) return;
    go(opensOnSetup ? { view: 'setup' } : viewRoute(defaultView), { replace: true });
  }, [route.view, defaultView, opensOnSetup]);
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
      const hit = await ticketByKey(key);
      if (hit) openTicket(hit);
    });
  }, [openTicket]);
  const pmSessions = pm.data?.sessions;
  const openSession = useCallback((s: { id: string; projectId: string }) => {
    // A PM conversation has no tab: the PM view shows it.
    if (pmSessions?.some((x) => x.session.id === s.id)) {
      setPmSelection(s.id);
      return;
    }
    void runAction(async () => {
      const item = { kind: 'session', id: s.id } as const;
      if (await ws.focusItem(item, s.projectId)) setView('sessions');
      // A session whose project has no workspace (a host-project workflow step's) opens in the workspace on screen.
      else if (ws.workspace) {
        await ws.openIn(item, ws.workspace.id);
        setView('sessions');
      }
    });
  }, [ws.focusItem, ws.openIn, ws.workspace, pmSessions, setPmSelection, setView]);
  // A new PM conversation starts beside the open ones (PDT-149): its first message starts it.
  const newPmSession = useCallback(() => {
    setSheetOpen(false);
    setPmSelection('new');
  }, [setPmSelection]);
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
  useRightPanelKey(() => (phone ? setTodosSheetOpen((o) => !o) : setRightOpen((o) => !o)));

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
    if (a.type === 'notes') {
      setView('notes');
      return;
    }
    if (a.type === 'todos') {
      if (phone) {
        setTodosSheetOpen(true);
        setTodoAddSeq((n) => n + 1);
      } else {
        if (!rightOpen) setTodoAddSeq((n) => n + 1);
        setRightOpen(!rightOpen);
      }
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

  const label = VIEWS.find((v) => v.id === view)!.label;
  const current = ws.currentInfo;
  const settingsProject = settingsPage.kind === 'project' ? ws.projects.find((p) => p.key === settingsPage.key) ?? null : null;
  const section = settingsPage.kind === 'host' || settingsPage.kind === 'project' ? settingsPage.section : null;
  const showSection = (slug: string) => {
    if (settingsPage.kind === 'host' || settingsPage.kind === 'project') go({ view: 'settings', page: { ...settingsPage, section: slug } });
  };
  const currentProject = ws.projects.find((p) => p.id === current?.projectId) ?? null;
  const worktrees = useQuery({
    queryKey: ['worktrees', currentProject?.id ?? ''],
    queryFn: () => api.listWorktrees(currentProject!.id),
    enabled: !!currentProject && !!current?.worktreeId,
  });
  const worktreeBranch = current?.worktreeId ? worktrees.data?.find((w) => w.id === current.worktreeId)?.branch : undefined;
  const branch = current?.run?.branch ?? worktreeBranch ?? null;
  const branchLink = currentProject?.forge && branch ? branchUrl(currentProject.forge, branch) : null;
  // The Files view: the file in the path, in the checkout the path names (a project key and worktree) or the
  // current tab's.
  const filesRoute = route.view === 'files' ? route : null;
  const filesProject = filesRoute?.project ? ws.projects.find((p) => p.key === filesRoute.project) : undefined;
  // A path ending in `/` is a folder: the phone shows it as its own screen (PDT-120).
  const filesDir = filesRoute?.path?.endsWith('/') ? filesRoute.path.slice(0, -1) : null;
  const folder: FolderViewState = useFolderView(ws.projects, current ? { projectId: current.projectId, worktreeId: current.worktreeId } : null, {
    picked: filesProject ? { projectId: filesProject.id, worktreeId: filesRoute?.worktree ?? null } : null,
    file: filesDir === null ? filesRoute?.path ?? null : null,
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

  /** Settings for a project (by id), plugins, all connections, secrets, notifications, or this host (null). */
  const showProjectSettings = (id: string | null) => {
    const key = ws.projects.find((p) => p.id === id)?.key;
    go({
      view: 'settings',
      page: id === PLUGINS ? { kind: 'plugins', id: null } : id === CONNECTIONS ? { kind: 'connections' } : id === SECRETS ? { kind: 'secrets' } : id === NOTIFICATIONS ? { kind: 'notifications' }
        : key ? { kind: 'project', key, section: null } : { kind: 'host', section: null },
    });
    setSheetOpen(false);
  };

  // Notifications (PDT-108): the Inbox's Updates, the stream's toasts and desktop notifications, and where a click goes.
  const updates = useUpdates();
  const readAllUpdates = useReadAllUpdates();
  const clearUpdate = useClearUpdate();
  const clearAllUpdates = useClearAllUpdates();
  const unreadUpdates = (updates.data?.unreadUpdates ?? 0) > 0;
  // The PM's unread reply: no dot for the conversation on screen (on a phone `/pm` is the list).
  const pmShown = route.view !== 'pm' || (phone && pmSelection === 'current') ? null
    : shownConversation(pm.data?.sessions, pm.data?.current?.id, pmSelection)?.session.id ?? null;
  const pmDot = pmUnread(pm.data?.sessions, pmShown);
  useAttentionTitle(useAttention(ws.everyInfo, pmShown));
  const openTarget = useCallback((t: NotificationTarget) => {
    switch (t.kind) {
      case 'session': openSession({ id: t.id, projectId: t.projectId }); break;
      case 'ticket': openTicket({ id: t.id, projectId: t.projectId }); break;
      case 'ticket_key': openTicketByKey(t.key); break;
      case 'inbox': go({ view: 'inbox', itemId: t.id }); break;
      case 'workflow': setWorkflowTarget({ kind: 'workflow', id: t.id }); break;
      case 'url': window.open(t.url, '_blank', 'noopener'); break;
      case 'connections': go({ view: 'settings', page: { kind: 'connections' } }); break;
      case 'todo':
        if (phone) setTodosSheetOpen(true);
        else setRightOpen(true);
        setTodoFocus({ id: t.id, seq: Date.now() });
        break;
      case 'none': break;
    }
  }, [openSession, openTicket, openTicketByKey, setWorkflowTarget, phone, setRightOpen]);
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
  const issueForge = useProjectForge(ticketProjectId ?? null).data;
  const issueRepo = issueForge?.issues ? issueForge.repo : null;
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
        const s = await api.createSession({ ...newSession(at.projectId, at.worktreeId, isAcp(kind) ? kind : CHOICE[kind]), ...(tab ? { tab } : {}) });
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
    reportBug: () => setReportingBug(true),
    showFiles: () => setView('files'),
    showWorkflows: () => setView('workflows'),
    showInbox: () => setView('inbox'),
    showPersonas: () => setView('personas'),
    showPm: () => setView('pm'),
    showNotes: () => setView('notes'),
    newTicket: () => setNewTicket({ projectId: ticketProjectId, title: '' }),
    newIssue: () => { if (ticketProjectId && issueRepo) setNewIssue({ projectId: ticketProjectId, repo: issueRepo }); },
    openTicket,
    startTicket: (t, here) => startTicket(t, { here }),
    startNextPart: (t) => void runAction(() => startNextPart(startTicket, t)),
  };
  const agents = useAgentAvailability();
  const defaults = useAgentDefaults();
  const acpAgents = useAcpAgents();
  const items = paletteOpen
    ? paletteItems({
        workspaces: ws.workspaces, infos: [...ws.allInfos, ...ws.ticketInfos], projects: ws.projects, tab: ws.tab, place, theme, handlers,
        tickets: looksLikeKey(keyQuery) ? ticketHits.data?.items ?? [] : [],
        ticketProjectId: view === 'tickets' ? backlog.projectId : null,
        issueRepo,
        agents: agents.of,
        defaults,
        acpAgents,
      })
    : [];

  const list: ReactNode = view === 'sessions' ? (
    // The Sidebar has its own section headings (Workspaces, Agents).
    <>
      {ws.projects.length === 0 && (
        <EmptyState action={<Button size="sm" onClick={() => { setSheetOpen(false); setAddingProject(true); }}>Add a project</Button>}>
          No projects yet.
        </EmptyState>
      )}
      <Sidebar
        ws={ws}
        onRenameTab={setRenamingTabId}
        onProjectSettings={showProjectSettings}
        onChoose={() => setSheetOpen(false)}
        touch={phone}
        adding={addingWorkspace}
        onAddingChange={setAddingWorkspace}
      />
    </>
  ) : view === 'inbox' ? (
    <InboxList items={inboxItems} updates={updates.data?.items ?? []} selectedId={inboxId} loading={inbox.isLoading}
      onSelect={(id) => { setInboxId(id); setSheetOpen(false); }}
      onReadAll={() => readAllUpdates.mutate()} onClear={(id) => clearUpdate.mutate(id)} onClearAll={() => clearAllUpdates.mutate()}
      onSettings={() => showProjectSettings(NOTIFICATIONS)} />
  ) : view === 'pm' ? (
    <PmList selection={pmSelection} onSelect={(sel) => { setPmSelection(sel); setSheetOpen(false); }} onNew={newPmSession} />
  ) : view === 'notes' ? (
    <NotesList filters={notes} selectedId={noteId} onOpen={openNote} onNew={newNote} onWhose={(w) => go(notesList(w))} />
  ) : view === 'documents' ? (
    <DocumentsList filters={documents} selectedId={docId} onOpen={openDoc} onNew={() => setDocDialog('new')} onImport={() => setDocDialog('import')} />
  ) : view === 'personas' ? (
    <PersonasList selectedId={personaId} onSelect={(id) => { setPersonaId(id); setSheetOpen(false); }} />
  ) : view === 'tickets' ? (
    <TicketsList backlog={backlog} projects={ws.projects} onChoose={() => setSheetOpen(false)} />
  ) : view === 'workflows' ? (
    <WorkflowsList projects={ws.projects} current={workflowTarget}
      onOpen={(t) => { setWorkflowTarget(t); setSheetOpen(false); }}
      onNew={() => { setSheetOpen(false); setCreatingWorkflow({}); }} />
  ) : view === 'files' ? (
    phone ? <FolderList state={folder} dir="" /> : (
      <>
        <h2 className={listHeading}>{label}</h2>
        <FolderTree state={folder} />
      </>
    )
  ) : (
    <SettingsNav
      projects={ws.projects}
      page={settingsPage}
      phone={phone}
      heading={<h2 className={listHeading}>{label}</h2>}
      onGo={(page) => { go({ view: 'settings', page }); setSheetOpen(false); }}
      onAddProject={() => { setSheetOpen(false); setAddingProject(true); }}
    />
  );
  // The phone's screens (PDT-112): a view's top-level screen (its list, for most views) or a detail screen with ←.
  const detail = isDetail(route);
  const phoneList = phone && !detail && !setupShown && PHONE_LIST_SCREENS.includes(view);
  // The overview and a rename in the top bar belong to the workspace screen they were opened on.
  const workspaceScreen = view === 'sessions' && detail ? wsId : null;
  useEffect(() => {
    setOverviewOpen(false);
    setRenamingWorkspace(false);
  }, [workspaceScreen]);
  const pickerSheet = phone && view === 'tickets';
  const ticketsProject = ws.projects.find((p) => p.id === backlog.projectId);
  // A workspace screen's ⋯ (phones): its tabs, its ticket, and (a free-form or ticket workspace) Rename and Close.
  const shownWorkspace = ws.workspace;
  const workspaceActions = (): MenuEntry[] => {
    const w = shownWorkspace;
    const free = !!w && w.projectId === null;
    return [
      { label: 'New tab', onSelect: () => setChooserOpen(true), disabled: !w },
      { label: 'Tab overview', onSelect: () => setOverviewOpen(true), disabled: !w },
      ...(w?.ticket ? [{ label: `Open ticket ${w.ticket.key}`, onSelect: () => go({ view: 'tickets', board: false, ticket: w.ticket!.key }) }] : []),
      ...(free ? [{ label: 'Rename', onSelect: () => setRenamingWorkspace(true) }] : []),
      { label: bottomOpen ? 'Hide terminal panel' : 'Terminal panel', onSelect: () => setBottomOpen((o) => !o) },
      ...(free ? [{ separator: true } as const, { label: 'Close workspace…', danger: true, onSelect: () => closeWorkspace(w) }] : []),
    ];
  };
  // Closing a workspace keeps its tabs: they go back to their projects' workspaces.
  const closeWorkspace = (w: Workspace) => void runAction(async () => {
    const { ok } = await confirm({
      title: `Close ${w.name}?`,
      body: 'Its tabs move back to their projects’ workspaces; nothing is stopped.',
      action: 'Close workspace',
    });
    if (!ok) return;
    await ws.deleteWorkspace(w.id);
    goBack(routePath(viewRoute('sessions')));
  });
  // A conversation with the host's default agent, in the drawer's project (else the first), in its main checkout.
  const newConversation = () => {
    const projectId = ws.projects.find((p) => p.id === stored.projectId)?.id ?? ws.projects[0]?.id;
    if (!projectId) return;
    void runAction(async () => {
      const s = await api.createSession(newSession(projectId, null, choiceFor(defaults.agent, 'conversation'), { permissionMode: defaults.sessionPermissionMode }));
      void qc.invalidateQueries({ queryKey: ['sessions', s.projectId] });
      await ws.refresh();
      await ws.focusItem({ kind: 'session', id: s.id }, s.projectId);
    });
  };
  const phoneTitle: ReactNode = setupShown ? 'Get started' : view === 'tickets' && !detail ? (
    <Button variant="ghost" size="sm" aria-label="Projects and filters" aria-haspopup="dialog" onClick={() => setSheetOpen(true)} className="min-w-0 font-medium">
      <span className="truncate">{ticketsProject ? `${label} · ${ticketsProject.key}` : label}</span>
      <ChevronDown aria-hidden className="size-4 text-muted-foreground" />
    </Button>
  ) : view === 'tickets' && detail ? peekRef ?? label
  : view === 'inbox' && detail && inboxId ? (triageTitle(inboxItems, inboxId) ?? label)
  : view === 'sessions' && detail && renamingWorkspace && shownWorkspace ? (
    <InlineRename
      label="Workspace name"
      initial={shownWorkspace.name}
      className="w-full"
      onCommit={(name) => { setRenamingWorkspace(false); void runAction(() => ws.renameWorkspace(shownWorkspace.id, name)); }}
      onCancel={() => setRenamingWorkspace(false)}
    />
  ) : view === 'sessions' && detail ? (shownWorkspace?.ticket ? `${shownWorkspace.ticket.key} ${shownWorkspace.ticket.title}` : shownWorkspace?.name ?? label)
  : view === 'files' && detail && filesRoute?.path ? filesRoute.path.split('/').filter(Boolean).pop() ?? label
  : view === 'settings' && detail ? settingsTitle()
  : view === 'pm' && detail ? (pmSelection === 'earlier' ? 'Ended conversations' : pmSelection === 'new' ? 'New conversation'
    : shownConversation(pm.data?.sessions, pm.data?.current?.id, pmSelection)?.title ?? label)
  : view === 'notes' && detail ? (noteId === 'new' ? 'New note' : 'Note')
  : view === 'documents' && detail ? 'Document'
  : label;
  function settingsTitle(): string {
    switch (settingsPage.kind) {
      case 'host': return (section && sectionLabel(HOST_SECTION_LABELS, section)) || 'This host';
      case 'project': return (section && sectionLabel(PROJECT_SECTION_LABELS, section)) || (settingsProject?.name ?? settingsPage.key);
      case 'index': return label;
      default: return settingsPageTitle(settingsPage, settingsContext);
    }
  }
  const shellActions = (): MenuEntry[] =>
    view === 'pm' ? [
      { label: 'New conversation', onSelect: newPmSession },
      { label: 'Ended conversations', onSelect: () => setPmSelection('earlier') },
    ] : view === 'sessions' && detail ? workspaceActions()
    : view === 'sessions' ? [
      { label: 'New workspace', onSelect: () => setAddingWorkspace(true) },
      { label: 'New conversation', onSelect: newConversation, disabled: ws.projects.length === 0 },
    ] : [];
  const hasShellActions = !setupShown && (view === 'pm' || view === 'sessions');
  const phoneActions: TopBarActions | null = hasShellActions || screenActions
    ? () => [...shellActions(), ...(screenActions?.entries() ?? [])]
    : null;
  // The todo list's "current project": the workspace on screen's, else the one picked in the lists.
  const todoProjectId = (view === 'sessions' ? ws.workspace?.projectId : null) ?? stored.projectId ?? null;
  const projectKeyList = ws.projects.map((p) => p.key).join(' ');
  const projectKeys = useMemo(() => (projectKeyList ? projectKeyList.split(' ') : []), [projectKeyList]);
  const tabPanelOf = view === 'sessions' && !phoneList && !setupShown && !(phone && overviewOpen) ? ws.tab?.id ?? null : null;

  return (
    <TicketKeysContext.Provider value={projectKeys}>
    <OpenTicketContext.Provider value={openTicket}>
    <OpenSessionContext.Provider value={openSession}>
    <OpenChangesContext.Provider value={openChanges}>
    <TicketTabsContext.Provider value={ticketTabs}>
    <StartTicketContext.Provider value={startTicket}>
    <TopBarActionsContext.Provider value={setScreenActions}>
    <TopBarEditContext.Provider value={setScreenEdit}>
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
            onBack={() => goBack(routePath(route.view === 'notes' ? notesList(notes.whose) : upRoute(route)))}
            title={phoneTitle}
            needsYou={inbox.data?.total ?? 0}
            onInbox={() => go(viewRoute('inbox'))}
            onSearch={() => setPaletteOpen(true)}
            onTodos={() => setTodosSheetOpen(true)}
            actions={phoneActions}
            edit={screenEdit}
          />
          <PhoneDrawer
            open={drawerOpen}
            onOpenChange={setDrawerOpen}
            view={view}
            onView={setView}
            counts={{ inbox: inbox.data?.total ?? 0 }}
            dots={{ inbox: unreadUpdates, pm: pmDot }}
            ws={ws}
            projectId={stored.projectId}
            onProject={stored.setProjectId}
            onRenameTab={setRenamingTabId}
            theme={theme}
            onTheme={setTheme}
          />
          <Sheet side="right" title="Todos" hideTitle open={todosSheetOpen} onOpenChange={setTodosSheetOpen} className={cn('flex flex-col', todoOpenInSheet ? 'w-screen max-w-none' : 'w-80 max-w-[90vw]')}>
            <RightPanelContent
              projectId={todoProjectId}
              projects={ws.projects}
              focusTodo={todoFocus}
              onTodoFocused={clearTodoFocus}
              focusAddSeq={todoAddSeq}
              onTodoOpen={setTodoOpenInSheet}
            />
          </Sheet>
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
            dots={{ inbox: unreadUpdates, pm: pmDot }}
            theme={{ value: theme, onChange: setTheme }}
          />
          <aside aria-label={`${label} list`} className={cn('flex min-h-0 w-72 flex-col border-r border-border', (view === 'notes' || view === 'documents') && 'w-80')}>
            <div className="flex-1 overflow-auto px-1">{list}</div>
          </aside>
        </>
      )}
      <main className="relative flex-1 flex flex-col min-w-0 min-h-0">
        <UpdateBanner />
        {view === 'sessions' && !phoneList && !setupShown ? (
          <WorkspaceBar
            ws={ws}
            renamingTabId={renamingTabId}
            setRenamingTabId={setRenamingTabId}
            onNewTab={() => setChooserOpen(true)}
            {...(phone ? { onOverview: () => setOverviewOpen(true) } : {})}
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
        {view === 'sessions' && !phoneList && !setupShown && mergedTicketId && (
          <Suspense fallback={null}><WorkspaceMergedBanner key={mergedTicketId} ticketId={mergedTicketId} /></Suspense>
        )}
        {chooserOpen && view === 'sessions' && <NewTabChooser ws={ws} onClose={() => setChooserOpen(false)} anchor={plusRef} />}
        {helpOpen && <KeyHelp onClose={() => setHelpOpen(false)} onReportBug={() => { setHelpOpen(false); setReportingBug(true); }} />}
        {reportingBug && (
          <BugReportDialog onClose={() => setReportingBug(false)} onOpenSettings={() => { setReportingBug(false); showProjectSettings(null); }} />
        )}
        <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} items={items} onQueryChange={setPaletteQuery} />
        {newTicket && (
          <Suspense fallback={null}><NewTicketDialog
            projects={ws.projects}
            projectId={newTicket.projectId}
            title={newTicket.title}
            onClose={() => setNewTicket(null)}
          /></Suspense>
        )}
        {docDialog === 'new' && (
          <NewDocumentDialog projects={ws.projects} projectId={documents.project || null} onClose={() => setDocDialog(null)}
            onCreated={(d) => { setDocDialog(null); openDoc(d.id); }} />
        )}
        {docDialog === 'import' && (
          <ImportDocumentDialog projectId={documents.project || null} onClose={() => setDocDialog(null)}
            onImported={(d) => { setDocDialog(null); openDoc(d.id); }} />
        )}
        {newIssue && (
          <Suspense fallback={null}><CreateIssueDialog projectId={newIssue.projectId} repo={newIssue.repo} onClose={() => setNewIssue(null)} /></Suspense>
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
            mode={addMode}
            onClose={() => { setAddingProject(false); setAddMode('path'); }}
            onAdded={(p) => {
              // Get started stays on screen: its project step ticks off.
              if (setupShown) {
                void qc.invalidateQueries({ queryKey: ['host-setup'] });
                return;
              }
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
          <div
            className="flex-1 flex flex-col min-h-0"
            // Under the workspace's tab strip, this is the current tab's panel.
            {...(tabPanelOf ? { role: 'tabpanel', 'aria-labelledby': workspaceTabId(tabPanelOf) } : {})}
          >
            <ViewScopeContext.Provider value={tabPanelOf ? tabScope(tabPanelOf) : `view:${view}`}>
            <ErrorBoundary resetKey={loc} onBack={() => go(viewRoute('inbox'))}>
            {setupShown ? (
              <SetupPage
                onAddProject={(mode) => { setAddMode(mode ?? 'path'); setAddingProject(true); }}
                onBoard={() => go({ view: 'tickets', board: true, ticket: null })}
                onPm={() => setView('pm')}
                onHostSettings={(section) => go({ view: 'settings', page: { kind: 'host', section } })}
                onDismissed={() => go(viewRoute(defaultView))}
              />
            ) : phone && view === 'sessions' && !phoneList && overviewOpen ? (
              <TabOverview ws={ws} onClose={() => setOverviewOpen(false)} onRename={setRenamingTabId} />
            ) : phoneList && view === 'inbox' ? (
              <PhoneInboxList items={inboxItems} updates={updates.data?.items ?? []} loading={inbox.isLoading}
                onOpen={(id) => go({ view: 'inbox', itemId: id })}
                onReadAll={() => readAllUpdates.mutate()} onClear={(id) => clearUpdate.mutate(id)} onClearAll={() => clearAllUpdates.mutate()}
                onSettings={() => showProjectSettings(NOTIFICATIONS)} />
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
              <ScreenSuspense><PhoneInboxItem items={inboxItems} updates={updates.data?.items ?? []}
                itemId={inboxId} loading={inbox.isLoading} onMove={moveInbox} onList={() => goBack(routePath(viewRoute('inbox')))} onOpenTarget={openTarget} /></ScreenSuspense>
            ) : view === 'inbox' ? (
              <ScreenSuspense><InboxView items={inboxItems} updates={updates.data?.items ?? []} selectedId={inboxId} onSelect={setInboxId} onOpenTarget={openTarget} /></ScreenSuspense>
            ) : view === 'pm' ? (
              <ScreenSuspense><PmView selection={pmSelection} onSelect={setPmSelection} onNew={newPmSession} /></ScreenSuspense>
            ) : view === 'notes' ? (
              noteId ? (
                <ScreenSuspense><NoteScreen
                  id={noteId}
                  whose={notes.whose}
                  scope={notes.scope}
                  phone={phone}
                  onCreated={(id) => go({ view: 'notes', memory: false, note: id }, { replace: true })}
                  onGone={() => (phone ? goBack(routePath(notesList(notes.whose))) : go(notesList(notes.whose), { replace: true }))}
                  onKind={setNotesWhose}
                /></ScreenSuspense>
              ) : (
                <EmptyState icon={<NotebookPen />} className="mt-16 border-0" action={<Button size="sm" onClick={newNote}>New note</Button>}>
                  {notes.whose === 'note' ? 'Pick a note from the PM’s memory, or add one.' : 'Pick a note, or start a new one.'}
                </EmptyState>
              )
            ) : view === 'documents' ? (
              docId ? (
                <ScreenSuspense><DocumentPage
                  key={docId}
                  id={docId}
                  phone={phone}
                  onGone={() => (phone ? goBack(routePath(viewRoute('documents'))) : go(viewRoute('documents'), { replace: true }))}
                /></ScreenSuspense>
              ) : (
                <EmptyState icon={<FileText />} className="mt-16 border-0" action={<Button size="sm" onClick={() => setDocDialog('new')}>New document</Button>}>
                  Pick a document, or make one.
                </EmptyState>
              )
            ) : view === 'personas' ? (
              <ScreenSuspense><PersonaPage key={personaId ?? 'none'} personaId={personaId} onSelect={setPersonaId} /></ScreenSuspense>
            ) : view === 'tickets' ? (
              <ScreenSuspense><TicketsView backlog={backlog} projects={ws.projects} peek={peek} onNewTicket={(projectId, title) => setNewTicket({ projectId, title: title ?? '' })} /></ScreenSuspense>
            ) : view === 'workflows' ? (
              <ScreenSuspense><WorkflowsView target={workflowTarget} onOpen={setWorkflowTarget} projects={ws.projects} projectInView={currentProject?.id ?? null} creating={creatingWorkflow} onCreate={setCreatingWorkflow} /></ScreenSuspense>
            ) : view === 'files' ? (
              phone && filesDir !== null ? <div className="flex-1 overflow-auto px-1"><FolderList state={folder} dir={filesDir} /></div> : <FileViewer state={folder} readOnly={phone} />
            ) : view === 'settings' ? (
              settingsPage.kind === 'project' ? (
                settingsProject ? <ProjectPage key={settingsProject.id} project={settingsProject} onRemoved={() => (phone ? go(viewRoute('settings'), { replace: true }) : showProjectSettings(null))}
                  drill={phone} section={section} onSection={showSection}
                  onKeyChange={(key) => go({ view: 'settings', page: { kind: 'project', key, section } }, { replace: true })} />
                  : ws.projectsLoaded ? <EmptyState>No project has the key {settingsPage.key}.</EmptyState> : <Skeleton lines={8} className="p-4" />
              ) : settingsPage.kind === 'host' || settingsPage.kind === 'index' ? <HostSettingsPage drill={phone} section={section} onSection={showSection} />
                : settingsPageScreen(settingsPage, settingsContext)
            ) : ws.tab?.ticketId ? (
              <ScreenSuspense><TicketView key={ws.tab.ticketId} ticketId={ws.tab.ticketId} mergedBanner={ws.tab.ticketId !== mergedTicketId} onDeleted={() => void ws.refresh()} /></ScreenSuspense>
            ) : ws.tab?.worktreeId ? (
              <ScreenSuspense><ChangesView key={ws.tab.worktreeId} worktreeId={ws.tab.worktreeId} /></ScreenSuspense>
            ) : current?.run ? (
              <RunView
                key={current.run.id}
                run={current.run}
                onRemoved={() => void ws.refresh()}
                branchLink={branchLink}
                {...(currentProject ? { forge: currentProject.forge ?? null, baseBranch: currentProject.baseBranch } : {})}
              />
            ) : current?.session ? (
              <SessionView
                key={current.session.id}
                session={current.session}
                onRemoved={() => void ws.refresh()}
                branchLink={branchLink}
                {...(currentProject ? { forge: currentProject.forge ?? null, baseBranch: currentProject.baseBranch, projectKey: currentProject.key } : {})}
                {...(!current.worktreeId ? { branch: null } : worktreeBranch !== undefined ? { branch: worktreeBranch } : {})}
              />
            ) : ws.projects.length === 0 ? (
              <EmptyState><Link onClick={() => go({ view: 'setup' })}>Add a project to get started.</Link></EmptyState>
            ) : (
              <EmptyState>{ws.tab ? 'Loading…' : 'No tabs here. Press + to start an agent or a shell (Ctrl+] c).'}</EmptyState>
            )}
            </ErrorBoundary>
            </ViewScopeContext.Provider>
          </div>
        )}
        {bottomOpen && <BottomPanel project={bottomProject} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
      {!phone && <RightPanel open={rightOpen} onOpenChange={setRightOpen} projectId={todoProjectId} projects={ws.projects} focusTodo={todoFocus} onTodoFocused={clearTodoFocus} focusAddSeq={todoAddSeq} />}
    </div>
    </TopBarEditContext.Provider>
    </TopBarActionsContext.Provider>
    </StartTicketContext.Provider>
    </TicketTabsContext.Provider>
    </OpenChangesContext.Provider>
    </OpenSessionContext.Provider>
    </OpenTicketContext.Provider>
    </TicketKeysContext.Provider>
  );
}
