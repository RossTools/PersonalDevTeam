import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, GitBranch, Play, TestTube, Ticket } from 'lucide-react';
import { Alert, Button, CommandList, Dialog, Popover, Sheet, Switch, type CommandItem } from '@ui';
import { api } from '../api/client';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { TabPlacement } from '../api/gen/TabPlacement';
import { AgentDot, itemState } from '../common/AgentState';
import { PlacePicker } from '../common/PlacePicker';
import { useAcpAgents } from '../common/agents';
import { agentIcon } from '../common/SessionKindIcon';
import { runAction } from '../common/useAction';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { useMediaQuery } from '../common/useMediaQuery';
import { PermissionModeSelect } from '../conversation/ConversationComposer';
import { RunBranchForm } from '../runs/RunBranchForm';
import { choiceLabel, parseChoice, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { acpChoices, agentOf, byDefault, choiceText, newSession, type Choice } from '../sessions/newSession';
import { TicketPicker } from '../tickets/TicketPicker';
import { findTab } from './model';
import type { WorkspacesApi } from './useWorkspaces';

const KEYS: Record<string, Choice> = { c: 'Claude', t: 'Claude (terminal)', x: 'Codex', o: 'Codex (terminal)', d: 'Devin', e: 'Devin (terminal)', s: 'Shell' };
const AGENTS: Choice[] = ['Claude', 'Claude (terminal)', 'Codex', 'Codex (terminal)', 'Devin', 'Devin (terminal)', 'Shell'];
const iconOf = (c: Choice) => agentIcon(agentOf(c));
/** ACP agents have no key. */
const keyOf = (c: Choice): string | undefined => Object.keys(KEYS).find((k) => KEYS[k] === c);

/** Where a new session starts: the user's pick, else the current tab's place, else the project workspace's project. */
function usePlace(ws: WorkspacesApi) {
  const initial = ws.currentInfo
    ? { projectId: ws.currentInfo.projectId, worktreeId: ws.currentInfo.worktreeId }
    : ws.workspace?.projectId
      ? { projectId: ws.workspace.projectId, worktreeId: null }
      : null;
  const [picked, setPicked] = useState<{ projectId: string; worktreeId: string | null } | null>(null);
  const place = picked ?? initial ?? (ws.projects[0] ? { projectId: ws.projects[0].id, worktreeId: null } : null);
  return { place, setPicked };
}

/** True below Tailwind's `sm` breakpoint (phones), where the chooser is a bottom sheet. */
const useNarrow = () => useMediaQuery('(max-width: 639.98px)');

/** The overlay the chooser lives in: a popover under `anchor` (a bottom sheet on phones), else a dialog. */
function Frame({ anchor, onClose, children }: { anchor?: RefObject<Element | null>; onClose: () => void; children: ReactNode }) {
  const narrow = useNarrow();
  const onOpenChange = (o: boolean) => !o && onClose();
  if (anchor && narrow) {
    return (
      <Sheet side="bottom" title="New tab" hideTitle open onOpenChange={onOpenChange}>
        {children}
      </Sheet>
    );
  }
  if (anchor) {
    return (
      <Popover anchor={anchor} label="New tab" open onOpenChange={onOpenChange} placement="bottom start" className="w-96 p-1">
        {children}
      </Popover>
    );
  }
  return (
    <Dialog open onOpenChange={onOpenChange} title="New tab" size="sm">
      {children}
    </Dialog>
  );
}

/**
 * The new-tab chooser: one filterable list to launch a Claude, Codex, Devin or ACP agent conversation, an agent's terminal or a
 * shell (c / t / x / o / d / e / s while the filter is empty), offering only the agents this host has (all of them
 * until the host checks load; a logged-out one says so), the host's default agent and view first,
 * start a configured run or test, run another branch, or bring an existing session or run here. With `anchor` (the
 * tab strip's "+") it is a popover there, a bottom sheet on phones; without one, a dialog.
 */
export function NewTabChooser({ ws, onClose, anchor }: { ws: WorkspacesApi; onClose: () => void; anchor?: RefObject<Element | null> }) {
  const qc = useQueryClient();
  const agents = useAgentAvailability();
  const offered = (c: Choice) => { const a = agentOf(c); return !a || agents.offered(a); };
  // The host's default agent and view first.
  const defaults = useAgentDefaults();
  const acp = useAcpAgents();
  // The ACP agents' conversations go before Shell.
  const all = [...AGENTS.slice(0, -1), ...acpChoices(acp), 'Shell' as const];
  const choicesHere = byDefault(all.filter(offered), (c) => c, defaults);
  const { place, setPicked } = usePlace(ws);
  // The host's default for new sessions (Ask unless changed) until picked here.
  const [pickedBypass, setBypass] = useState<boolean | null>(null);
  const [pickedMode, setPermissionMode] = useState<PermissionMode | null>(null);
  const permissionMode = pickedMode ?? defaults.sessionPermissionMode;
  const bypass = pickedBypass ?? defaults.sessionPermissionMode === 'bypass';
  const [query, setQuery] = useState('');
  const [view, setView] = useState<'list' | 'branch' | 'ticket'>('list');
  const workspace = ws.workspace;
  const placement: TabPlacement | undefined = workspace
    ? { workspaceId: workspace.id, position: ws.tab ? ws.tab.position + 1 : workspace.tabs.length }
    : undefined;
  const done = async (item: { kind: 'session' | 'run'; id: string }, projectId: string) => {
    await ws.refresh();
    await ws.focusItem(item, projectId);
    onClose();
  };
  const create = useMutation({
    mutationFn: (choice: Choice) => api.createSession({ ...newSession(place!.projectId, place!.worktreeId, choice, { bypass, permissionMode }), ...(placement ? { tab: placement } : {}) }),
    onSuccess: async (s) => {
      void qc.invalidateQueries({ queryKey: ['sessions', s.projectId] });
      await done({ kind: 'session', id: s.id }, s.projectId);
    },
  });
  const { choices, error: configError } = useRunChoices(place?.projectId ?? '', place?.worktreeId ?? null);
  const start = useStartRun(place?.projectId ?? '', (runId) => void done({ kind: 'run', id: runId }, place!.projectId));
  const others = ws.allInfos.flatMap((i) => {
    const where = findTab(ws.workspaces, i.item)?.workspace;
    return where?.id === workspace?.id ? [] : [{ info: i, workspaceName: where?.name ?? null }];
  });
  const project = ws.projects.find((p) => p.id === place?.projectId) ?? null;
  const error = create.error ?? start.error;

  const items: CommandItem[] = [
    ...(place
      ? choicesHere.map((c) => {
          const Icon = iconOf(c);
          const hint = agentOf(c) ? agents.hint(agentOf(c)!) : undefined;
          const shortcut = keyOf(c);
          return { id: `new:${c}`, label: choiceText(c, acp), ...(hint ? { detail: hint } : {}), group: 'Start here', ...(shortcut ? { shortcut } : {}), icon: <Icon aria-hidden />, disabled: create.isPending };
        })
      : []),
    ...(place && workspace ? [{ id: 'ticket', label: 'Ticket…', group: 'Start here', icon: <Ticket aria-hidden /> }] : []),
    ...(place
      ? choices.map((c) => ({
          id: `choice:${c}`,
          label: c.startsWith('test:') ? choiceLabel(c) : `Run ${choiceLabel(c)}`,
          group: 'Run',
          icon: c.startsWith('test:') ? <TestTube aria-hidden /> : <Play aria-hidden />,
          disabled: start.isPending,
        }))
      : []),
    ...(project && choices.length > 0
      ? [{ id: 'branch', label: 'Run another branch…', group: 'Run', icon: <GitBranch aria-hidden /> }]
      : []),
    ...others.map(({ info, workspaceName }) => ({
      id: `bring:${info.item.kind}:${info.item.id}`,
      label: info.title,
      ...(workspaceName ? { detail: workspaceName } : {}),
      group: 'Bring here',
      icon: <AgentDot state={itemState(info)} />,
      keywords: [info.session?.agent ?? info.item.kind],
    })),
  ];

  const onAction = (id: string) => {
    const [what, ...rest] = id.split(':');
    const value = rest.join(':');
    if (what === 'new' && place && !create.isPending) create.mutate(value as Choice);
    else if (what === 'choice' && place) {
      start.mutate({ ...(place.worktreeId ? { worktreeId: place.worktreeId } : {}), ...parseChoice(value), ...(placement ? { tab: placement } : {}) });
    } else if (what === 'branch') setView('branch');
    else if (what === 'ticket') setView('ticket');
    else if (what === 'bring') {
      const hit = others.find((o) => `${o.info.item.kind}:${o.info.item.id}` === value);
      if (hit && workspace) void runAction(() => ws.openIn(hit.info.item, workspace.id, placement?.position)).then(onClose);
    }
  };

  // c / t / x / o / d / e / s launch Claude, Claude (terminal), Codex, Codex (terminal), Devin, Devin (terminal) or a
  // shell (an agent the host lacks does nothing) while focus is in the chooser, outside any text field, and the filter is empty; any other printable key moves to the filter and types there. Overlays own their element, so listen on
  // window and check where the key came from.
  const marker = useRef<HTMLDivElement>(null);
  const onKey = useRef<(e: KeyboardEvent) => void>(() => {});
  onKey.current = (e) => {
    const target = e.target as HTMLElement | null;
    const panel = marker.current?.closest('[role=dialog]');
    if (view !== 'list' || !panel || !target || target.closest('[role=dialog]') !== panel) return;
    // Not while typing, nor on type-ahead in a list or picker (their popovers are portalled outside the panel, so the
    // check above already skips them; this also covers any rendered inline).
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || e.ctrlKey || e.metaKey || e.altKey) return;
    if (target.closest('[role=listbox],[role=option],[role=menu],[role=menuitem],[role=grid],[role=combobox]')) return;
    if (e.key.length !== 1 || e.key === ' ') return;
    const choice = KEYS[e.key];
    if (choice && query === '' && place) {
      e.preventDefault();
      if (!create.isPending && offered(choice)) create.mutate(choice);
      return;
    }
    // Type-ahead: the key lands in the filter (focusing it during keydown sends the character there).
    marker.current?.querySelector<HTMLInputElement>('[data-slot=command-input]')?.focus();
  };
  useEffect(() => {
    const listener = (e: KeyboardEvent) => onKey.current(e);
    window.addEventListener('keydown', listener);
    return () => window.removeEventListener('keydown', listener);
  }, []);

  return (
    <Frame anchor={anchor} onClose={onClose}>
      <div ref={marker} className="flex min-h-0 flex-col gap-2">
        {place ? (
          <PlacePicker projects={ws.projects} place={place} onPick={setPicked} className="self-start" />
        ) : (
          <p className="text-sm text-muted-foreground">Add a project first.</p>
        )}
        {view === 'ticket' && place && workspace ? (
          <div className="flex flex-col gap-1">
            <Button variant="ghost" size="sm" className="self-start" onClick={() => setView('list')}>
              <ArrowLeft aria-hidden className="size-4" /> Back
            </Button>
            <TicketPicker
              projectId={place.projectId}
              label="Ticket to open here"
              onPick={(t) => void runAction(() => ws.openIn({ kind: 'ticket', id: t.id }, workspace.id, placement?.position)).then(onClose)}
              className="max-h-80"
            />
          </div>
        ) : view === 'branch' && project ? (
          <div className="flex flex-col gap-1">
            <Button variant="ghost" size="sm" className="self-start" onClick={() => setView('list')}>
              <ArrowLeft aria-hidden className="size-4" /> Back
            </Button>
            <RunBranchForm project={project} onSelectRun={(runId) => void done({ kind: 'run', id: runId }, project.id)} />
          </div>
        ) : (
          <CommandList
            label="New tab choices"
            placeholder={`Filter, or press ${choicesHere.flatMap((c) => keyOf(c) ?? []).join(', ').replace(/, (\w)$/, ' or $1')}…`}
            items={items}
            onAction={onAction}
            query={query}
            onQueryChange={setQuery}
            autoFocus={false}
          />
        )}
        {configError && <p className="text-meta text-failed">{configError}</p>}
        {error && <Alert tone="error">{error.message}</Alert>}
        <div className="flex flex-col gap-1.5 border-t border-border pt-2">
          <div className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-2 text-xs">
              <span aria-hidden className="text-muted-foreground">Conversation permissions</span>
              <PermissionModeSelect value={permissionMode} onChange={setPermissionMode} label="Permission mode for a new conversation" />
            </span>
            <span className="shrink-0 text-meta text-muted-foreground pointer-coarse:hidden">Esc to close</span>
          </div>
          <Switch size="sm" checked={bypass} onChange={setBypass} aria-label="Skip permission prompts for a new terminal agent" className="text-xs">
            Terminal agents: skip permission prompts (dangerous)
          </Switch>
        </div>
      </div>
    </Frame>
  );
}
