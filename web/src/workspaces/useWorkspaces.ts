import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useQueries, useQuery, useQueryClient, type Query, type QueryClient } from '@tanstack/react-query';
import { useConfirm, type Status } from '@ui';
import { api } from '../api/client';
import type { Activity } from '../api/gen/Activity';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import type { Session } from '../api/gen/Session';
import type { Tab } from '../api/gen/Tab';
import type { Ticket } from '../api/gen/Ticket';
import type { TabPlacement } from '../api/gen/TabPlacement';
import type { Workspace } from '../api/gen/Workspace';
import type { Worktree } from '../api/gen/Worktree';
import { isActive, runDot } from '../runs/runState';
import { hiddenStepSession, toDot } from '../sessions/status';
import { sessionUnseen } from '../common/AgentState';
import { useSameList } from '../common/useSameList';
import { pruneTabViewStates } from '../common/viewState';
import { worktreesQuery } from '../common/worktrees';
import { findTab, itemOf, openTabBody, resolveCurrent, type CurrentChoice, type Item } from './model';
import { useProjects } from '../projects/queries';
import { invalidateWorkspaces, workspaceKeys, workspacesQuery } from './queries';
import { invalidateSessions, sessionsQuery } from '../sessions/queries';
import { invalidateRuns, runsQuery } from '../runs/queries';
import { ticketKeys } from '../tickets/queries';
import { POLL } from '../common/poll';

const STORAGE_KEY = 'pdt.workspaces.current';

function loadChoice(): CurrentChoice {
  try {
    const v = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null');
    if (v && typeof v === 'object' && typeof v.tabs === 'object') return { workspaceId: v.workspaceId ?? null, tabs: v.tabs ?? {} };
  } catch {
    // Unreadable or blocked storage: start from the first workspace.
  }
  return { workspaceId: null, tabs: {} };
}

function saveChoice(c: CurrentChoice) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(c));
  } catch {
    // Storage blocked (private window): the choice lasts for this page only.
  }
}

/** What the UI needs to show an item in a tab or the tree. */
export interface ItemInfo {
  item: Item;
  title: string;
  status: Status;
  activity: Activity | null;
  unseen: boolean;
  projectId: string;
  worktreeId: string | null;
  session?: Session;
  run?: Run;
  ticket?: Ticket;
}

export function sessionInfo(s: Session): ItemInfo {
  return {
    item: { kind: 'session', id: s.id }, title: s.title, status: toDot(s.status), activity: s.activity, unseen: sessionUnseen(s),
    projectId: s.projectId, worktreeId: s.worktreeId, session: s,
  };
}

export function runInfo(r: Run): ItemInfo {
  return {
    item: { kind: 'run', id: r.id }, title: `${r.kind === 'test' ? 'Test ' : ''}${r.name} · ${r.branch}`,
    status: runDot(r.state), activity: null, unseen: false, projectId: r.projectId, worktreeId: r.worktreeId, run: r,
  };
}

/**
 * A ticket tab: titled `PDT-42 · title`, no agent state (the tab shows a ticket icon). Its place is its active
 * attempt's worktree, so a new session or run from it (and the folder view) defaults to the ticket's branch.
 */
export function ticketInfo(t: Ticket): ItemInfo {
  const worktreeId = t.attempt?.state === 'active' ? t.attempt.worktreeId : null;
  return {
    item: { kind: 'ticket', id: t.id }, title: `${t.key} · ${t.title}`, status: 'idle', activity: null, unseen: false,
    projectId: t.projectId, worktreeId, ticket: t,
  };
}

/** A changes tab: titled `Changes · <branch>`. */
export function changesInfo(w: Worktree): ItemInfo {
  return {
    item: { kind: 'changes', id: w.id }, title: `Changes · ${w.branch}`, status: 'idle', activity: null, unseen: false,
    projectId: w.projectId, worktreeId: w.id,
  };
}

/**
 * The workspace and tab a route names (PDT-111), and how a selection reaches the route. A route's choice wins over the
 * stored one (and is stored, for when a route doesn't say); `replace` is for a selection the user didn't make (the tab
 * after a closed one).
 */
export interface RoutedChoice {
  workspaceId: string | null;
  tabId: string | null;
  onChoose: (workspaceId: string, tabId: string | null, opts: { replace: boolean }) => void;
}

/**
 * A ticket tab's poll: its title every 5 s, none while another view (the ticket's own, `useTicket`, every 2 s) watches the same ticket, so
 * it isn't fetched twice (PDT-451). The count is read again when [`useTicketWatchers`] re-renders.
 */
export const ticketTabPoll = (q: Query) => (q.getObserversCount() > 1 ? false : POLL.steady);

/**
 * Re-renders when a view starts or stops watching one of `ids` (the tabs' tickets), so their polls ([`ticketTabPoll`])
 * follow it; other tickets' views don't re-render the shell.
 */
function useTicketWatchers(qc: QueryClient, ids: string[]) {
  const subscribe = useCallback(
    (changed: () => void) => qc.getQueryCache().subscribe((e) => {
      if ((e.type === 'observerAdded' || e.type === 'observerRemoved') && e.query.queryKey[0] === ticketKeys.one('')[0]) changed();
    }),
    [qc],
  );
  const watched = () =>
    ids.filter((id) => (qc.getQueryCache().find({ queryKey: ticketKeys.one(id), exact: true })?.getObserversCount() ?? 0) > 1).join(',');
  useSyncExternalStore(subscribe, watched);
}

/**
 * Workspaces, the per-device choice of workspace and tab, every project's sessions and runs (the same queries the
 * tree uses), and the tab actions.
 */
export function useWorkspaces(routed?: RoutedChoice) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const projects = useProjects();
  const projectList: Project[] = useMemo(() => projects.data ?? [], [projects.data]);
  const workspacesQ = useQuery(workspacesQuery);
  const workspaces: Workspace[] = useMemo(() => workspacesQ.data ?? [], [workspacesQ.data]);
  // What a closed tab's views remembered goes with it (closed here or on another device).
  useEffect(() => {
    if (workspacesQ.data) pruneTabViewStates(workspacesQ.data.flatMap((w) => w.tabs.map((t) => t.id)));
  }, [workspacesQ.data]);
  const sessionQs = useQueries({ queries: projectList.map((p) => sessionsQuery(p.id)) });
  const runQs = useQueries({ queries: projectList.map((p) => runsQuery(p.id)) });

  // Query data keeps its identity while polls bring nothing new, so these change only when something did (reading
  // `dataUpdatedAt` would make every consumer re-render on every poll, PDT-136).
  const sessionData = useSameList(sessionQs.map((q) => q.data));
  const runData = useSameList(runQs.map((q) => q.data));
  const infos = useMemo(() => {
    const m = new Map<string, ItemInfo>();
    for (const d of sessionData) for (const s of d ?? []) m.set(`session:${s.id}`, sessionInfo(s));
    for (const d of runData) for (const r of d ?? []) m.set(`run:${r.id}`, runInfo(r));
    return m;
  }, [sessionData, runData]);
  // Ticket tabs: one query per ticket on a tab (shared with the ticket's own view, `ticketKeys.one`).
  const ticketIds = useMemo(
    () => [...new Set(workspaces.flatMap((w) => w.tabs.flatMap((t) => (t.ticketId ? [t.ticketId] : []))))].sort(),
    [workspaces],
  );
  useTicketWatchers(qc, ticketIds);
  const ticketQs = useQueries({
    queries: ticketIds.map((id) => ({ queryKey: ticketKeys.one(id), queryFn: () => api.getTicket(id), refetchInterval: ticketTabPoll })),
  });
  const ticketData = useSameList(ticketQs.map((q) => q.data));
  const ticketInfos = useMemo(() => ticketData.flatMap((t) => (t ? [ticketInfo(t)] : [])), [ticketData]);
  // Changes tabs: their worktrees, from the projects' worktree lists (read only while such a tab exists).
  const hasChangesTab = workspaces.some((w) => w.tabs.some((t) => t.worktreeId));
  const worktreeQs = useQueries({
    queries: projectList.map((p) => ({ ...worktreesQuery(p.id), enabled: hasChangesTab })),
  });
  const worktreeData = useSameList(worktreeQs.map((q) => q.data));
  const changesInfos = useMemo(() => worktreeData.flatMap((d) => (d ?? []).map(changesInfo)), [worktreeData]);
  const info = useCallback(
    (item: Item) =>
      item.kind === 'ticket'
        ? ticketInfos.find((i) => i.item.id === item.id) ?? null
        : item.kind === 'changes'
          ? changesInfos.find((i) => i.item.id === item.id) ?? null
          : infos.get(`${item.kind}:${item.id}`) ?? null,
    [infos, ticketInfos, changesInfos],
  );
  // Every session and run, hidden ones included: what the favicon and title watch (PDT-262).
  const everyInfo = useMemo(() => [...infos.values()], [infos]);
  // The palette's and Bring here's sessions and runs: a workflow step's session (a ticket's, or an ended one with no
  // ticket) only while a tab holds it (PDT-263, PDT-290). The Agents list leaves out every step's session, tab or not
  // (`startedByStep`, PDT-299).
  const allInfos = useMemo(
    () => everyInfo.filter((i) => !i.session || !hiddenStepSession(i.session) || findTab(workspaces, i.item)),
    [everyInfo, workspaces],
  );

  const [stored, setChoiceState] = useState<CurrentChoice>(loadChoice);
  const setChoice = useCallback((f: (c: CurrentChoice) => CurrentChoice) => {
    setChoiceState((c) => {
      const next = f(c);
      saveChoice(next);
      return next;
    });
  }, []);
  const routedWs = routed?.workspaceId ?? null;
  const routedTab = routed?.tabId ?? null;
  const choice: CurrentChoice = useMemo(
    () => (routedWs ? { workspaceId: routedWs, tabs: routedTab ? { ...stored.tabs, [routedWs]: routedTab } : stored.tabs } : stored),
    [routedWs, routedTab, stored],
  );
  // The route's choice is remembered for the next time a route doesn't name one.
  useEffect(() => {
    if (routedWs && (stored.workspaceId !== routedWs || (routedTab && stored.tabs[routedWs] !== routedTab))) setChoice(() => choice);
  }, [routedWs, routedTab, stored, choice, setChoice]);
  const onChoose = useRef(routed?.onChoose);
  onChoose.current = routed?.onChoose;
  const choiceRef = useRef(choice);
  choiceRef.current = choice;
  const { workspace, tab } = resolveCurrent(workspaces, choice);

  const selectWorkspace = useCallback((id: string) => {
    setChoice((c) => ({ ...c, workspaceId: id }));
    onChoose.current?.(id, choiceRef.current.tabs[id] ?? null, { replace: false });
  }, [setChoice]);
  const selectTab = useCallback((t: Tab) => {
    setChoice((c) => ({ workspaceId: t.workspaceId, tabs: { ...c.tabs, [t.workspaceId]: t.id } }));
    onChoose.current?.(t.workspaceId, t.id, { replace: false });
  }, [setChoice]);
  const refresh = useCallback(() => invalidateWorkspaces(qc), [qc]);

  /** Shows an item: its tab wherever it is, else a new tab in its project's workspace. False when it has no home. */
  const focusItem = useCallback(
    async (item: Item, projectId: string): Promise<boolean> => {
      const cached = qc.getQueryData<Workspace[]>(workspaceKeys.all) ?? [];
      let found = findTab(cached, item);
      if (!found) {
        const fresh = await qc.fetchQuery({ ...workspacesQuery, staleTime: 0 });
        found = findTab(fresh, item);
        if (!found) {
          const home = fresh.find((w) => w.projectId === projectId);
          if (!home) return false;
          const t = await api.openTab(home.id, openTabBody(item));
          await refresh();
          selectTab(t);
          return true;
        }
      }
      selectTab(found.tab);
      return true;
    },
    [qc, refresh, selectTab],
  );

  /** Where a session or run launched from the tree opens: after the current tab when its project's workspace is on
   * screen, else (undefined) the end of that workspace. */
  const placementFor = useCallback(
    (projectId: string): TabPlacement | undefined => {
      if (!workspace || workspace.projectId !== projectId) return undefined;
      return { workspaceId: workspace.id, position: tab ? tab.position + 1 : workspace.tabs.length };
    },
    [workspace, tab],
  );

  /** Closes a tab. A session or run goes with its tab (the daemon stops and removes it), so a running one asks first. */
  const closeTab = useCallback(
    async (t: Tab) => {
      const i = info(itemOf(t));
      const running = i?.session ? i.session.status === 'running' : i?.run ? isActive(i.run.state) : false;
      if (running && i) {
        const what = i.session ? (i.session.kind === 'agent_tui' ? 'agent' : 'session') : 'run';
        const { ok } = await confirm({
          title: `Close ${i.title}?`,
          body: i.session
            ? `Closing the tab stops the ${what} and removes it: its terminal history is deleted and it can no longer be resumed. Files in its folder stay.`
            : 'Closing the tab stops the run and removes it. Its data folder stays.',
          action: 'Stop and close',
          danger: true,
        });
        if (!ok) return;
      }
      // Closed first: a close that fails leaves the tab where it was, still selected.
      await api.closeTab(t.id);
      const w = workspaces.find((x) => x.id === t.workspaceId);
      if (w && tab?.id === t.id) {
        const i = w.tabs.findIndex((x) => x.id === t.id);
        const neighbour = w.tabs[i + 1] ?? w.tabs[i - 1];
        setChoice((c) => ({ ...c, tabs: { ...c.tabs, [w.id]: neighbour?.id ?? '' } }));
        onChoose.current?.(w.id, neighbour?.id ?? null, { replace: true });
      }
      await refresh();
      if (t.sessionId) void invalidateSessions(qc);
      if (t.runId) void invalidateRuns(qc);
    },
    [info, confirm, workspaces, tab, setChoice, refresh, qc],
  );

  const moveTab = useCallback(
    async (t: Tab, workspaceId: string, position?: number) => {
      await api.updateTab(t.id, { workspaceId, ...(position !== undefined ? { position } : {}) });
      await refresh();
    },
    [refresh],
  );

  /** Opens an item in a workspace (moving its tab if it has one) and shows it. */
  const openIn = useCallback(
    async (item: Item, workspaceId: string, position?: number) => {
      const t = await api.openTab(workspaceId, {
        ...openTabBody(item),
        ...(position !== undefined ? { position } : {}),
      });
      await refresh();
      selectTab(t);
    },
    [refresh, selectTab],
  );

  const createWorkspace = useCallback(
    async (name: string) => {
      const w = await api.createWorkspace({ name });
      await refresh();
      selectWorkspace(w.id);
      return w;
    },
    [refresh, selectWorkspace],
  );
  const renameWorkspace = useCallback(async (id: string, name: string) => { await api.updateWorkspace(id, { name }); await refresh(); }, [refresh]);
  const moveWorkspace = useCallback(async (id: string, position: number) => { await api.updateWorkspace(id, { position }); await refresh(); }, [refresh]);
  const deleteWorkspace = useCallback(async (id: string) => { await api.deleteWorkspace(id); await refresh(); }, [refresh]);

  const currentInfo = tab ? info(itemOf(tab)) : null;
  const projectsLoaded = projects.isSuccess;
  const workspacesError = workspacesQ.error;
  // One object while nothing in it changed, so the shell's other polls (the Inbox, the PM, updates) don't re-render the
  // views it is passed to (PDT-454).
  return useMemo(() => ({
    createWorkspace, renameWorkspace, moveWorkspace, deleteWorkspace,
    projects: projectList, projectsLoaded, workspaces, workspacesError, workspace, tab, info, allInfos, everyInfo, ticketInfos,
    selectWorkspace, selectTab, focusItem, placementFor, closeTab, moveTab, openIn, refresh, currentInfo,
  }), [
    createWorkspace, renameWorkspace, moveWorkspace, deleteWorkspace,
    projectList, projectsLoaded, workspaces, workspacesError, workspace, tab, info, allInfos, everyInfo, ticketInfos,
    selectWorkspace, selectTab, focusItem, placementFor, closeTab, moveTab, openIn, refresh, currentInfo,
  ]);
}

export type WorkspacesApi = ReturnType<typeof useWorkspaces>;
