import { enc, request } from './core';
import type { BranchNamesInput } from './gen/BranchNamesInput';
import type { BranchNamesPreview } from './gen/BranchNamesPreview';
import type { BranchNamesView } from './gen/BranchNamesView';

/** A project's branch naming: the template new tickets' branches get, its preview, and the setting on this host. */
export const branchNamesApi = {
  branchNames: (projectId: string) => request<BranchNamesView>(`/api/projects/${enc(projectId)}/branch-names`),
  saveBranchNames: (projectId: string, s: BranchNamesInput) =>
    request<BranchNamesView>(`/api/projects/${enc(projectId)}/branch-names`, { method: 'PUT', body: JSON.stringify(s) }),
  previewBranchNames: (projectId: string, s: BranchNamesInput) =>
    request<BranchNamesPreview>(`/api/projects/${enc(projectId)}/branch-names/preview`, { method: 'POST', body: JSON.stringify(s) }),
};
