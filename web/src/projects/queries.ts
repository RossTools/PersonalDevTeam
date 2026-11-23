// Projects: the list every screen shares, and what a project write marks stale.
import { useQuery, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';
import { workspaceKeys } from '../workspaces/queries';

export const projectKeys = {
  all: ['projects'] as const,
  branches: (projectId: string) => ['branches', projectId] as const,
  branchNames: (projectId: string) => ['branch-names', projectId] as const,
  branchNamesPreview: (projectId: string, template: string, types: string) => ['branch-names', projectId, 'preview', template, types] as const,
  settings: (projectId: string) => ['project-settings', projectId] as const,
  settingsSchemas: ['project-settings-schemas'] as const,
  /** What worktrees take on disk: every project's (null) or one project's. */
  storage: (projectId: string | null) => ['storage', projectId] as const,
  storageAll: ['storage'] as const,
  /** On the owner: a project's remote instances and the pairing code on show. */
  remoteInstances: (projectId: string) => ['remote-instances', projectId] as const,
  /** On a remote instance: the projects it joined. */
  remote: ['remote-projects'] as const,
};

/** This instance's remote projects (other instances' projects it works for), polled for their link's state. */
export function useRemoteProjects() {
  return useQuery({ queryKey: projectKeys.remote, queryFn: api.listRemoteProjects, refetchInterval: POLL.steady });
}

/** A project's remote instances, polled for their heartbeats (and the pairing code's expiry). */
export function useRemoteInstances(projectId: string) {
  return useQuery({ queryKey: projectKeys.remoteInstances(projectId), queryFn: () => api.remoteInstances(projectId), refetchInterval: POLL.steady });
}

/** Every project. They change through this UI (which invalidates them), so a mount within 30 s doesn't refetch. */
export function useProjects({ enabled = true }: { enabled?: boolean } = {}) {
  return useQuery({ queryKey: projectKeys.all, queryFn: api.listProjects, staleTime: 30_000, enabled });
}

/** After a write that changes what worktrees take on disk, or the storage settings: every storage view. */
export function invalidateStorage(qc: QueryClient) {
  return qc.invalidateQueries({ queryKey: projectKeys.storageAll });
}

/** After a project is added, changed or removed: the list, and the workspaces (each project has one). */
export function invalidateProjects(qc: QueryClient) {
  const projects = qc.invalidateQueries({ queryKey: projectKeys.all });
  void qc.invalidateQueries({ queryKey: workspaceKeys.all });
  return projects;
}
