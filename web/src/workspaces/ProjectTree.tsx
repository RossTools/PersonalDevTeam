import { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import type { Project } from '../api/gen/Project';
import { SessionList } from '../sessions/SessionList';
import { findTab } from './model';
import type { WorkspacesApi } from './useWorkspaces';
import { AttentionDots, summarize } from './WorkspaceBar';

const STORAGE_KEY = 'pdt.tree.collapsed';

function loadCollapsed(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((x) => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

/** Every project, collapsible (remembered per device), with its places, sessions and runs. */
export function ProjectTree({ ws, onOpenSettings }: { ws: WorkspacesApi; onOpenSettings: (projectId: string) => void }) {
  const [collapsed, setCollapsed] = useState<string[]>(loadCollapsed);
  const toggle = (id: string) => {
    const next = collapsed.includes(id) ? collapsed.filter((x) => x !== id) : [...collapsed, id];
    setCollapsed(next);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Storage blocked: remembered for this page only.
    }
  };
  const current = ws.currentInfo?.item ?? null;
  const whereIs = (kind: 'session' | 'run', id: string) => {
    const w = findTab(ws.workspaces, { kind, id })?.workspace;
    return w && w.projectId === null ? w.name : null;
  };

  if (ws.projects.length === 0) return <p className="px-2 text-xs text-muted">No projects yet.</p>;
  return (
    <div className="flex flex-col gap-1">
      {ws.projects.map((p: Project) => {
        const open = !collapsed.includes(p.id);
        const s = summarize(ws.allInfos.filter((i) => i.projectId === p.id));
        return (
          <section key={p.id} aria-label={`Project ${p.name}`}>
            <button
              type="button"
              aria-expanded={open}
              onClick={() => toggle(p.id)}
              title={p.repoPath}
              className="w-full flex items-center gap-1.5 px-1 py-1 rounded-pdt text-sm font-medium hover:bg-surface-2"
            >
              {open ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}
              <span className="flex-1 truncate text-left">{p.name}</span>
              <AttentionDots activity={s.activity} unseen={s.unseen} />
            </button>
            {open && (
              <div className="pl-2">
                <SessionList
                  project={p}
                  selectedId={current?.kind === 'session' ? current.id : null}
                  onSelect={(id) => void ws.focusItem({ kind: 'session', id }, p.id)}
                  selectedRunId={current?.kind === 'run' ? current.id : null}
                  onSelectRun={(id) => void ws.focusItem({ kind: 'run', id }, p.id)}
                  onOpenSettings={() => onOpenSettings(p.id)}
                  placement={() => ws.placementFor(p.id)}
                  whereIs={whereIs}
                />
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}
