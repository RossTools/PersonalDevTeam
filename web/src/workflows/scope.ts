import type { Project } from '../api/gen/Project';

/** Where a workflow applies, as the list groups it: This host, Personal, or the project's name. */
export function scopeLabel(w: { scope: string; projectId: string | null }, projects: Project[]): string {
  if (w.scope === 'personal') return 'Personal';
  if (w.scope === 'host') return 'This host';
  return projects.find((p) => p.id === w.projectId)?.name ?? 'A project';
}
