import { enc, request } from './core';
import type { Branch } from './gen/Branch';
import type { CloneProject } from './gen/CloneProject';
import type { NewProject } from './gen/NewProject';
import type { NewWorktree } from './gen/NewWorktree';
import type { Project } from './gen/Project';
import type { ProjectAreas } from './gen/ProjectAreas';
import type { ProjectPatch } from './gen/ProjectPatch';
import type { ProjectSettingsInput } from './gen/ProjectSettingsInput';
import type { ProjectSettingsView } from './gen/ProjectSettingsView';
import type { PullBranch } from './gen/PullBranch';
import type { PullResult } from './gen/PullResult';
import type { Worktree } from './gen/Worktree';

/** Projects, their worktrees, branches, labels, areas and settings. */
export const projectsApi = {
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
  cloneProject: (p: CloneProject) => request<Project>('/api/projects/clone', { method: 'POST', body: JSON.stringify(p) }),
  updateProject: (id: string, p: ProjectPatch) => request<Project>(`/api/projects/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  pullBranch: (id: string, p: PullBranch = {}) =>
    request<PullResult>(`/api/projects/${enc(id)}/pull`, { method: 'POST', body: JSON.stringify(p) }),
  deleteProject: (id: string) => request<void>(`/api/projects/${enc(id)}`, { method: 'DELETE' }),
  removeWorktree: (id: string, opts: { force?: boolean; deleteBranch?: boolean } = {}) =>
    request<void>(`/api/worktrees/${enc(id)}?force=${opts.force ? 'true' : 'false'}&deleteBranch=${opts.deleteBranch ? 'true' : 'false'}`, { method: 'DELETE' }),
  listWorktrees: (projectId: string) => request<Worktree[]>(`/api/projects/${enc(projectId)}/worktrees`),
  createWorktree: (projectId: string, w: NewWorktree) =>
    request<Worktree>(`/api/projects/${enc(projectId)}/worktrees`, { method: 'POST', body: JSON.stringify(w) }),
  projectLabels: (projectId: string) => request<string[]>(`/api/projects/${enc(projectId)}/labels`),
  /** The areas `.pdt/project.toml` (and the host overrides) declare, or why there are none. */
  getProjectAreas: (projectId: string) => request<ProjectAreas>(`/api/projects/${enc(projectId)}/areas`),
  listBranches: (projectId: string) => request<Branch[]>(`/api/projects/${enc(projectId)}/branches`),
  projectSettings: (projectId: string) => request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`),
  saveProjectSettings: (projectId: string, s: ProjectSettingsInput) =>
    request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`, { method: 'PUT', body: JSON.stringify(s) }),
};
