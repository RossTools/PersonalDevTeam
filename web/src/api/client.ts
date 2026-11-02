// The API client. Each area's calls live in its own module (projects.ts, tickets.ts, ...), the shared fetch and
// error handling in core.ts. A new call goes in its area's module; a new area adds its import and one line to `api`.
// `api` stays one plain object (tests spy on `api.foo`), and every area's keys must be unique across areas.

import { boardApi } from './board';
import { connectionsApi } from './connections';
import { documentsApi } from './documents';
import { filesApi } from './files';
import { forgesApi } from './forges';
import { hostApi } from './host';
import { inboxApi } from './inbox';
import { personasApi } from './personas';
import { pmApi } from './pm';
import { projectsApi } from './projects';
import { reviewApi } from './review';
import { runsApi } from './runs';
import { secretsApi } from './secrets';
import { sessionsApi } from './sessions';
import { storageApi } from './storage';
import { ticketsApi } from './tickets';
import { todosApi } from './todos';
import { watchesApi } from './watches';
import { workflowsApi } from './workflows';
import { workspacesApi } from './workspaces';

export { ApiError } from './core';
export type { SessionPatch } from './sessions';

export const api = {
  ...boardApi,
  ...connectionsApi,
  ...documentsApi,
  ...filesApi,
  ...forgesApi,
  ...hostApi,
  ...inboxApi,
  ...personasApi,
  ...pmApi,
  ...projectsApi,
  ...reviewApi,
  ...runsApi,
  ...secretsApi,
  ...sessionsApi,
  ...storageApi,
  ...ticketsApi,
  ...todosApi,
  ...watchesApi,
  ...workflowsApi,
  ...workspacesApi,
};
