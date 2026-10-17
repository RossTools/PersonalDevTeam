import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Zap } from 'lucide-react';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { TabPlacement } from '../api/gen/TabPlacement';
import { RunBranchForm } from '../runs/RunBranchForm';
import { parseChoice, RunMenu, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { newSession, type Choice } from '../sessions/newSession';
import { findTab } from './model';
import { PlacePicker } from './PlacePicker';
import type { WorkspacesApi } from './useWorkspaces';

const KEYS: Record<string, Choice> = { c: 'Claude', x: 'Codex', s: 'Shell' };
const btn = 'rounded-pdt border border-border px-3 py-1.5 text-sm hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50';

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
  // Focus once, on open: grabbing it on every render would close an open dropdown each time the lists refresh.
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => dialog.current?.focus(), []);
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
  const error = create.error ?? start.error;
  const project = ws.projects.find((p) => p.id === place?.projectId) ?? null;

  return (
    <>
      <div className="fixed inset-0 z-10 bg-black/20" onClick={onClose} aria-hidden />
      <div
        role="dialog"
        aria-label="New tab"
        tabIndex={-1}
        ref={dialog}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
          const choice = KEYS[e.key];
          if (choice && !(e.target instanceof HTMLInputElement) && place && !create.isPending) {
            e.preventDefault();
            create.mutate(choice);
          }
        }}
        className="absolute z-20 left-2 right-2 md:left-auto md:right-auto md:w-96 top-12 rounded-pdt border border-border bg-surface shadow-lg p-3 flex flex-col gap-3 outline-none"
      >
        {!place ? (
          <p className="text-sm text-muted">Add a project first.</p>
        ) : (
          <>
            <PlacePicker projects={ws.projects} place={place} onPick={setPicked} />
            <div className="flex flex-wrap items-center gap-2">
              {(['Claude', 'Codex', 'Shell'] as Choice[]).map((c) => (
                <button key={c} type="button" className={btn} disabled={create.isPending} onClick={() => create.mutate(c)}>
                  {c} <kbd className="ml-1 text-[10px] text-muted">{Object.keys(KEYS).find((k) => KEYS[k] === c)}</kbd>
                </button>
              ))}
              <button
                type="button"
                role="switch"
                aria-checked={bypass}
                aria-label="Skip permission prompts for the new agent"
                title="Skip the agent's permission prompts (dangerous)"
                onClick={() => setBypass((b) => !b)}
                className={`rounded-pdt p-1 ${bypass ? 'text-failed' : 'text-muted'} hover:bg-surface-2`}
              >
                <Zap size={14} aria-hidden fill={bypass ? 'currentColor' : 'none'} />
              </button>
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
            {configError && <p className="text-[11px] text-failed">{configError}</p>}
            {project && (
              <details className="text-xs text-muted">
                <summary className="cursor-pointer">Run another branch…</summary>
                <RunBranchForm project={project} onSelectRun={(runId) => void done({ kind: 'run', id: runId }, project.id)} />
              </details>
            )}
          </>
        )}
        <div className="flex flex-col gap-1">
          <input
            aria-label="Find a session or run"
            placeholder="Bring a session or run here…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && others[0] && workspace) void ws.openIn(others[0].item, workspace.id, placement?.position).then(onClose);
            }}
            className="px-2 py-1 rounded-pdt text-sm bg-bg border border-border outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <ul aria-label="Other sessions and runs" className="flex flex-col gap-0.5 max-h-56 overflow-auto">
            {others.map((i) => (
              <li key={`${i.item.kind}:${i.item.id}`}>
                <button
                  type="button"
                  className="w-full flex items-center gap-2 text-left px-2 py-1 rounded-pdt text-sm hover:bg-surface-2"
                  onClick={() => workspace && void ws.openIn(i.item, workspace.id, placement?.position).then(onClose)}
                >
                  <StatusDot status={i.status} />
                  <span className="truncate">{i.title}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
        {error && <p role="alert" className="text-xs text-failed">{error.message}</p>}
      </div>
    </>
  );
}
