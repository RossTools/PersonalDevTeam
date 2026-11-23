import { enc, request } from './core';
import type { JoinRemoteProject } from './gen/JoinRemoteProject';
import type { PlacementSettings } from './gen/PlacementSettings';
import type { RemoteInstancePatch } from './gen/RemoteInstancePatch';
import type { RemoteInstancesView } from './gen/RemoteInstancesView';
import type { RemoteProjectPatch } from './gen/RemoteProjectPatch';
import type { RemoteProjectView } from './gen/RemoteProjectView';

/**
 * Remote projects: on the owner, a project's remote instances (pairing codes, heartbeats on demand, Remove); on a
 * remote instance, the projects it joined (join, Pause, Remove). On the owner, a project's placement rules too.
 */
export const remoteProjectsApi = {
  remoteInstances: (projectId: string) => request<RemoteInstancesView>(`/api/projects/${enc(projectId)}/remote-instances`),
  newPairingCode: (projectId: string) =>
    request<RemoteInstancesView>(`/api/projects/${enc(projectId)}/remote-instances/pairing`, { method: 'POST' }),
  cancelPairingCode: (projectId: string) =>
    request<void>(`/api/projects/${enc(projectId)}/remote-instances/pairing`, { method: 'DELETE' }),
  checkRemoteInstances: (projectId: string) =>
    request<RemoteInstancesView>(`/api/projects/${enc(projectId)}/remote-instances/check`, { method: 'POST' }),
  patchRemoteInstance: (projectId: string, linkId: string, p: RemoteInstancePatch) =>
    request<RemoteInstancesView>(`/api/projects/${enc(projectId)}/remote-instances/${enc(linkId)}`, {
      method: 'PATCH',
      body: JSON.stringify(p),
    }),
  removeRemoteInstance: (projectId: string, linkId: string) =>
    request<void>(`/api/projects/${enc(projectId)}/remote-instances/${enc(linkId)}`, { method: 'DELETE' }),
  placement: (projectId: string) => request<PlacementSettings>(`/api/projects/${enc(projectId)}/placement`),
  putPlacement: (projectId: string, s: PlacementSettings) =>
    request<PlacementSettings>(`/api/projects/${enc(projectId)}/placement`, { method: 'PUT', body: JSON.stringify(s) }),
  listRemoteProjects: () => request<RemoteProjectView[]>('/api/remote-projects'),
  joinRemoteProject: (j: JoinRemoteProject) =>
    request<RemoteProjectView>('/api/remote-projects', { method: 'POST', body: JSON.stringify(j) }),
  patchRemoteProject: (id: string, p: RemoteProjectPatch) =>
    request<RemoteProjectView>(`/api/remote-projects/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  leaveRemoteProject: (id: string) => request<void>(`/api/remote-projects/${enc(id)}`, { method: 'DELETE' }),
};
