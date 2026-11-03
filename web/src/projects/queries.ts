// Projects: the list every screen shares, and what a project write marks stale.
import { useQuery, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { workspaceKeys } from '../workspaces/queries';

export const projectKeys = {
  all: ['projects'] as const,
  branches: (projectId: string) => ['branches', projectId] as const,
  settings: (projectId: string) => ['project-settings', projectId] as const,
  settingsSchemas: ['project-settings-schemas'] as const,
};

/** Every project. They change through this UI (which invalidates them), so a mount within 30 s doesn't refetch. */
export function useProjects({ enabled = true }: { enabled?: boolean } = {}) {
  return useQuery({ queryKey: projectKeys.all, queryFn: api.listProjects, staleTime: 30_000, enabled });
}

/** After a project is added, changed or removed: the list, and the workspaces (each project has one). */
export function invalidateProjects(qc: QueryClient) {
  const projects = qc.invalidateQueries({ queryKey: projectKeys.all });
  void qc.invalidateQueries({ queryKey: workspaceKeys.all });
  return projects;
}
