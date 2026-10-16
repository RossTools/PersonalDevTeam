import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { NewSession } from '../api/gen/NewSession';
import type { Project } from '../api/gen/Project';
import { RunBranchForm } from '../runs/RunBranchForm';
import { RunsSection } from '../runs/RunsSection';
import { INITIAL_SIZE, toDot } from './status';

type Choice = 'Shell' | 'Claude' | 'Codex';
const CHOICES: Choice[] = ['Shell', 'Claude', 'Codex'];

function newSession(projectId: string, worktreeId: string | null, choice: Choice): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  return choice === 'Shell'
    ? { ...base, kind: 'shell', ...INITIAL_SIZE }
    : { ...base, kind: 'agent_tui', agent: choice === 'Claude' ? 'claude' : 'codex', ...INITIAL_SIZE };
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

export function SessionList({ project, selectedId, onSelect, selectedRunId, onSelectRun, onOpenSettings }: {
  project: Project;
  selectedId: string | null;
  onSelect: (id: string) => void;
  selectedRunId?: string | null;
  onSelectRun?: (id: string) => void;
  onOpenSettings?: () => void;
}) {
  const qc = useQueryClient();
  const worktrees = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  const sessions = useQuery({
    queryKey: ['sessions', project.id],
    queryFn: () => api.listSessions(project.id),
    refetchInterval: 2000,
  });
  const create = useMutation({
    mutationFn: api.createSession,
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
            {CHOICES.map((c) => (
              <button
                key={c}
                type="button"
                aria-label={`New ${c} in ${g.label}`}
                disabled={create.isPending}
                onClick={() => create.mutate(newSession(project.id, g.id, c))}
                className={small}
              >
                {c}
              </button>
            ))}
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
                    title="Double-click to rename"
                    onClick={() => onSelect(s.id)}
                    onDoubleClick={() => setRenamingId(s.id)}
                    onKeyDown={(e) => { if (e.key === 'F2') setRenamingId(s.id); }}
                    className={`w-full flex items-center gap-2 text-left px-3 py-1 rounded-pdt text-sm ${s.id === selectedId ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
                  >
                    <StatusDot status={toDot(s.status)} />
                    <span className="truncate">{s.title}</span>
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
            />
          )}
        </section>
      ))}
      {onSelectRun && <RunBranchForm project={project} onSelectRun={onSelectRun} />}
      {error && <p role="alert" className="px-2 text-xs text-failed">{error.message}</p>}
    </div>
  );
}
