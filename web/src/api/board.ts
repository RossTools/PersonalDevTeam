import { enc, request } from './core';
import type { BoardSettings } from './gen/BoardSettings';

/** A project's board settings on this host (the Done window). */
export const boardApi = {
  boardSettings: (projectId: string) => request<BoardSettings>(`/api/projects/${enc(projectId)}/board`),
  saveBoardSettings: (projectId: string, s: BoardSettings) =>
    request<BoardSettings>(`/api/projects/${enc(projectId)}/board`, { method: 'PUT', body: JSON.stringify(s) }),
};
