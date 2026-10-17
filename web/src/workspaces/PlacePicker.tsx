import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';

export type Place = { projectId: string; worktreeId: string | null };

/** Picks a project and a place in it (the main checkout or one of its worktrees). */
export function PlacePicker({ projects, place, onPick }: {
  projects: Project[];
  place: Place;
  onPick: (p: Place) => void;
}) {
  const worktrees = useQuery({ queryKey: ['worktrees', place.projectId], queryFn: () => api.listWorktrees(place.projectId) });
  const select = 'rounded-pdt bg-bg border border-border text-xs px-1 py-0.5';
  return (
    <div className="flex gap-2">
      <select aria-label="Project" className={select} value={place.projectId} onChange={(e) => onPick({ projectId: e.target.value, worktreeId: null })}>
        {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
      </select>
      <select aria-label="Place" className={select} value={place.worktreeId ?? ''} onChange={(e) => onPick({ ...place, worktreeId: e.target.value || null })}>
        <option value="">Repository</option>
        {(worktrees.data ?? []).filter((w) => !w.missing).map((w) => <option key={w.id} value={w.id}>{w.branch}</option>)}
      </select>
    </div>
  );
}
