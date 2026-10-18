import { useEffect, useRef, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Input, Kbd, List, ListRow, Switch } from '@ui';
import { api } from '../api/client';
import type { TabPlacement } from '../api/gen/TabPlacement';
import { AgentDot, itemState } from '../common/AgentState';
import { PlacePicker } from '../common/PlacePicker';
import { runAction } from '../common/useAction';
import { RunBranchForm } from '../runs/RunBranchForm';
import { parseChoice, RunMenu, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { newSession, type Choice } from '../sessions/newSession';
import { findTab } from './model';
import type { WorkspacesApi } from './useWorkspaces';

const KEYS: Record<string, Choice> = { c: 'Claude', x: 'Codex', s: 'Shell' };

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

/** The new-tab chooser: launch Claude, Codex or a shell, start a run, or bring an existing session or run here. */
export function NewTabChooser({ ws, onClose }: { ws: WorkspacesApi; onClose: () => void }) {
  const qc = useQueryClient();
  const { place, setPicked } = usePlace(ws);
  const [bypass, setBypass] = useState(false);
  const [filter, setFilter] = useState('');
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
    mutationFn: (choice: Choice) => api.createSession({ ...newSession(place!.projectId, place!.worktreeId, choice, bypass), ...(placement ? { tab: placement } : {}) }),
    onSuccess: async (s) => {
      void qc.invalidateQueries({ queryKey: ['sessions', s.projectId] });
      await done({ kind: 'session', id: s.id }, s.projectId);
    },
  });
  const { choices, error: configError } = useRunChoices(place?.projectId ?? '', place?.worktreeId ?? null);
  const start = useStartRun(place?.projectId ?? '', (runId) => void done({ kind: 'run', id: runId }, place!.projectId));
  const others = ws.allInfos.filter((i) => {
    const where = findTab(ws.workspaces, i.item);
    return where?.workspace.id !== workspace?.id && i.title.toLowerCase().includes(filter.trim().toLowerCase());
  });
  const bringHere = (key: string) => {
    const i = others.find((o) => `${o.item.kind}:${o.item.id}` === key);
    if (i && workspace) void runAction(() => ws.openIn(i.item, workspace.id, placement?.position)).then(onClose);
  };
  const error = create.error ?? start.error;
  const project = ws.projects.find((p) => p.id === place?.projectId) ?? null;

  // c / x / s launch Claude, Codex or a shell while focus is in the chooser but not in a text field. A key handler on
  // the dialog itself isn't possible (Dialog owns that element), so listen on window and check where the key came from.
  const marker = useRef<HTMLDivElement>(null);
  const launch = useRef<(e: KeyboardEvent) => void>(() => {});
  launch.current = (e) => {
    const target = e.target as HTMLElement | null;
    const dialog = marker.current?.closest('[role=dialog]');
    if (!dialog || !target || target.closest('[role=dialog]') !== dialog) return;
    // Not while typing, nor on type-ahead in a list or picker (their popovers are portalled outside the dialog, so the
    // check above already skips them; this also covers any rendered inline).
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || e.ctrlKey || e.metaKey || e.altKey) return;
    if (target.closest('[role=listbox],[role=option],[role=menu],[role=grid],[role=combobox]')) return;
    const choice = KEYS[e.key];
    if (choice && place && !create.isPending) {
      e.preventDefault();
      create.mutate(choice);
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => launch.current(e);
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title="New tab" size="sm">
      <div ref={marker} className="flex flex-col gap-3">
        {!place ? (
          <p className="text-sm text-muted-foreground">Add a project first.</p>
        ) : (
          <>
            <PlacePicker projects={ws.projects} place={place} onPick={setPicked} className="self-start" />
            <div className="flex flex-wrap items-center gap-2">
              {(['Claude', 'Codex', 'Shell'] as Choice[]).map((c) => (
                <Button key={c} disabled={create.isPending} onClick={() => create.mutate(c)}>
                  {c} <Kbd>{Object.keys(KEYS).find((k) => KEYS[k] === c)}</Kbd>
                </Button>
              ))}
              {choices.length > 0 && (
                <RunMenu
                  label="Start a run"
                  choices={choices}
                  value=""
                  disabled={start.isPending}
                  onChange={(c) => start.mutate({ ...(place.worktreeId ? { worktreeId: place.worktreeId } : {}), ...parseChoice(c), ...(placement ? { tab: placement } : {}) })}
                />
              )}
            </div>
            <Switch size="sm" checked={bypass} onChange={setBypass} aria-label="Skip permission prompts for the new agent" className="text-xs">
              Skip permission prompts (dangerous)
            </Switch>
            {configError && <p className="text-meta text-failed">{configError}</p>}
            {project && (
              <details className="text-xs text-muted-foreground">
                <summary className="cursor-pointer">Run another branch…</summary>
                <RunBranchForm project={project} onSelectRun={(runId) => void done({ kind: 'run', id: runId }, project.id)} />
              </details>
            )}
          </>
        )}
        <div className="flex flex-col gap-1">
          <Input
            aria-label="Find a session or run"
            placeholder="Bring a session or run here…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && others[0]) bringHere(`${others[0].item.kind}:${others[0].item.id}`);
            }}
          />
          <List aria-label="Other sessions and runs" onAction={bringHere} className="max-h-56 overflow-auto">
            {others.map((i) => (
              <ListRow
                key={`${i.item.kind}:${i.item.id}`}
                id={`${i.item.kind}:${i.item.id}`}
                title={i.title}
                leading={<AgentDot state={itemState(i)} />}
              />
            ))}
          </List>
        </div>
        {error && <Alert tone="error">{error.message}</Alert>}
      </div>
    </Dialog>
  );
}
