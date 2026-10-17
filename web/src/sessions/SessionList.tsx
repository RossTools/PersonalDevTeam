import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Zap } from 'lucide-react';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { NewSession } from '../api/gen/NewSession';
import type { Project } from '../api/gen/Project';
import type { TabPlacement } from '../api/gen/TabPlacement';
import { dragItem } from '../workspaces/dnd';
import { RunBranchForm } from '../runs/RunBranchForm';
import { RunsSection } from '../runs/RunsSection';
import { ActivityBadge } from './ActivityBadge';
import { INITIAL_SIZE, toDot } from './status';

export type Choice = 'Shell' | 'Claude' | 'Codex';
const CHOICES: Choice[] = ['Shell', 'Claude', 'Codex'];

export function newSession(projectId: string, worktreeId: string | null, choice: Choice, bypass: boolean): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  return choice === 'Shell'
    ? { ...base, kind: 'shell', ...INITIAL_SIZE }
    : {
        ...base,
        kind: 'agent_tui',
        agent: choice === 'Claude' ? 'claude' : 'codex',
        // Only sent when armed, so a normal launch stays a plain payload.
        ...(bypass ? { bypassPermissions: true } : {}),
        ...INITIAL_SIZE,
      };
}

/**
 * The Shell / Claude / Codex buttons for one place, with a toggle that arms "skip permissions" for the next agent
 * launched here (Claude's `--dangerously-skip-permissions`, Codex's `--dangerously-bypass-approvals-and-sandbox`).
 * Needed on mobile, where you can't answer the agents' permission prompts. Shells ignore it.
 */
function LaunchButtons({ projectId, group, disabled, onCreate }: {
  projectId: string;
  group: { id: string | null; label: string };
  disabled: boolean;
  onCreate: (s: NewSession) => void;
}) {
  const [bypass, setBypass] = useState(false);
  return (
    <>
      <button
        type="button"
        role="switch"
        aria-checked={bypass}
        aria-label={`Skip permission prompts for new agents in ${group.label}`}
        title="Skip the agent's permission prompts (dangerous). Needed on mobile, where you can't answer them."
        disabled={disabled}
        onClick={() => setBypass((b) => !b)}
        className={`${small} ${bypass ? 'text-failed' : ''}`}
      >
        <Zap size={12} aria-hidden fill={bypass ? 'currentColor' : 'none'} />
      </button>
      {CHOICES.map((c) => (
        <button
          key={c}
          type="button"
          aria-label={`New ${c} in ${group.label}${bypass && c !== 'Shell' ? ', skip permissions' : ''}`}
          disabled={disabled}
          onClick={() => onCreate(newSession(projectId, group.id, c, bypass))}
          className={small}
        >
          {c}
        </button>
      ))}
    </>
  );
}

function RenameInput({ initial, onSave, onCancel }: {
  initial: string;
  onSave: (title: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [done, setDone] = useState(false);
  const finish = (save: boolean) => {
    if (done) return;
    setDone(true);
    const t = value.trim();
    if (save && t && t !== initial) onSave(t);
    else onCancel();
  };
  return (
    <input
      autoFocus
      aria-label="Session name"
      value={value}
      maxLength={100}
      onChange={(e) => setValue(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') finish(true);
        else if (e.key === 'Escape') finish(false);
      }}
      className="w-full px-3 py-1 rounded-pdt text-sm bg-surface text-fg border border-border outline-none focus-visible:ring-2 focus-visible:ring-ring"
    />
  );
}

const small = 'rounded-pdt px-1.5 py-0.5 text-[11px] text-muted hover:text-fg hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function SessionList({ project, selectedId, onSelect, selectedRunId, onSelectRun, onOpenSettings, placement, whereIs }: {
  project: Project;
  selectedId: string | null;
  onSelect: (id: string) => void;
  selectedRunId?: string | null;
  onSelectRun?: (id: string) => void;
  onOpenSettings?: () => void;
  /** Where a session or run launched here opens its tab (default: the end of the project's workspace). */
  placement?: () => TabPlacement | undefined;
  /** The name of the free-form workspace holding a session or run, shown next to it. */
  whereIs?: (kind: 'session' | 'run', id: string) => string | null;
}) {
  const qc = useQueryClient();
  const worktrees = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  const sessions = useQuery({
    queryKey: ['sessions', project.id],
    queryFn: () => api.listSessions(project.id),
    refetchInterval: 2000,
  });
  const create = useMutation({
    mutationFn: (s: NewSession) => {
      const tab = placement?.();
      return api.createSession(tab ? { ...s, tab } : s);
    },
    onSuccess: (s) => {
      void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
      onSelect(s.id);
    },
  });

  const [renamingId, setRenamingId] = useState<string | null>(null);
  const rename = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) => api.renameSession(id, title),
    onSettled: () => void qc.invalidateQueries({ queryKey: ['sessions', project.id] }),
  });

  if (worktrees.isPending || sessions.isPending) return <p className="px-2 text-xs text-muted">Loading…</p>;
  if (worktrees.isError || sessions.isError) {
    return <p role="alert" className="px-2 text-xs text-failed">{(worktrees.error ?? sessions.error)?.message}</p>;
  }
  const groups = [
    { id: null as string | null, label: 'Repository' },
    ...worktrees.data.map((w) => ({ id: w.id as string | null, label: w.branch })),
  ];
  const error = create.error ?? rename.error;

  return (
    <div className="flex flex-col gap-2 py-2">
      {onOpenSettings && (
        <Button variant="ghost" className="self-start mx-2" onClick={onOpenSettings}>Project settings</Button>
      )}
      {groups.map((g) => (
        <section key={g.id ?? 'repo'} aria-label={g.label}>
          <div className="flex items-center gap-0.5 px-2 text-xs text-muted">
            <span className="flex-1 truncate font-medium" title={g.label}>{g.label}</span>
            <LaunchButtons projectId={project.id} group={g} disabled={create.isPending} onCreate={create.mutate} />
          </div>
          <ul className="flex flex-col gap-0.5">
            {sessions.data.filter((s) => s.worktreeId === g.id).map((s) => (
              <li key={s.id}>
                {renamingId === s.id ? (
                  <RenameInput
                    initial={s.title}
                    onSave={(title) => {
                      setRenamingId(null);
                      rename.mutate({ id: s.id, title });
                    }}
                    onCancel={() => setRenamingId(null)}
                  />
                ) : (
                  <button
                    type="button"
                    aria-current={s.id === selectedId ? 'true' : undefined}
                    draggable
                    onDragStart={(e) => dragItem(e, { kind: 'session', id: s.id }, project.id)}
                    title="Double-click to rename"
                    onClick={() => onSelect(s.id)}
                    onDoubleClick={() => setRenamingId(s.id)}
                    onKeyDown={(e) => { if (e.key === 'F2') setRenamingId(s.id); }}
                    className={`w-full flex items-center gap-2 text-left px-3 py-1 rounded-pdt text-sm ${s.id === selectedId ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
                  >
                    <StatusDot status={toDot(s.status)} />
                    <span className={`truncate ${s.unseen ? 'font-semibold' : ''}`}>{s.title}</span>
                    {whereIs?.('session', s.id) && <span className="shrink-0 text-[10px] text-muted">in {whereIs('session', s.id)}</span>}
                    <ActivityBadge activity={s.activity} unseen={s.unseen} />
                  </button>
                )}
              </li>
            ))}
          </ul>
          {onSelectRun && (
            <RunsSection
              project={project}
              worktreeId={g.id}
              label={g.label}
              selectedRunId={selectedRunId ?? null}
              onSelectRun={onSelectRun}
              placement={placement}
              whereIs={whereIs}
            />
          )}
        </section>
      ))}
      {onSelectRun && <RunBranchForm project={project} onSelectRun={onSelectRun} />}
      {error && <p role="alert" className="px-2 text-xs text-failed">{error.message}</p>}
    </div>
  );
}
