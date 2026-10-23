// The app's URL paths (PDT-111): what each one shows, and the path for a screen. Pure: router.ts does the history.
import type { PmSelection } from '../pm/usePm';
import type { WorkflowTab } from '../workflows/WorkflowPage';
import type { WorkflowTarget } from '../workflows/WorkflowsList';
import type { ViewId } from './views';

/** A Settings page: this host (and a section of it), connections, notifications, or a project by key. */
export type SettingsPage =
  | { kind: 'host'; section: string | null }
  | { kind: 'connections' }
  | { kind: 'notifications' }
  | { kind: 'project'; key: string; section: string | null };

/** What a path shows. `view: null` is `/` (or a path the app doesn't know): the default view. */
export type Route =
  | { view: null }
  | { view: 'pm'; pm: PmSelection }
  | { view: 'sessions'; workspaceId: string | null; tabId: string | null }
  | { view: 'inbox'; itemId: string | null }
  | { view: 'tickets'; board: boolean; ticket: string | null }
  | { view: 'workflows'; target: WorkflowTarget }
  | { view: 'personas'; personaId: string | null }
  /** A file in a checkout; `project` (a key) and `worktree` say which checkout when the user picked one. */
  | { view: 'files'; path: string | null; project: string | null; worktree: string | null }
  | { view: 'settings'; page: SettingsPage };

const WORKFLOW_TABS: readonly WorkflowTab[] = ['outline', 'runs', 'graph', 'text', 'versions'];

const enc = encodeURIComponent;

function dec(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Reads a path (and its query string) as a route. */
export function parseRoute(pathname: string, search = ''): Route {
  const seg = pathname.split('/').filter(Boolean).map(dec);
  const q = new URLSearchParams(search);
  const [head, a = null, b = null, c = null] = seg;
  switch (head) {
    case 'pm':
      return { view: 'pm', pm: a ?? 'current' };
    case 'sessions':
      return { view: 'sessions', workspaceId: a, tabId: a ? b : null };
    case 'inbox':
      return { view: 'inbox', itemId: a };
    case 'tickets':
      return { view: 'tickets', board: q.get('view') === 'board', ticket: a };
    case 'workflows': {
      if (!a) return { view: 'workflows', target: null };
      if (a === 'templates') return { view: 'workflows', target: b ? { kind: 'template', id: b } : null };
      if (b === 'runs' && c) return { view: 'workflows', target: { kind: 'workflow', id: a, tab: 'runs', runId: c } };
      const tab = WORKFLOW_TABS.find((t) => t === b);
      return { view: 'workflows', target: { kind: 'workflow', id: a, ...(tab ? { tab } : {}) } };
    }
    case 'personas':
      return { view: 'personas', personaId: a };
    case 'files':
      return { view: 'files', path: seg.length > 1 ? seg.slice(1).join('/') : null, project: q.get('project'), worktree: q.get('worktree') };
    case 'settings':
      if (a === 'connections') return { view: 'settings', page: { kind: 'connections' } };
      if (a === 'notifications') return { view: 'settings', page: { kind: 'notifications' } };
      if (a === 'projects' && b) return { view: 'settings', page: { kind: 'project', key: b, section: c } };
      return { view: 'settings', page: { kind: 'host', section: a === 'host' ? b : null } };
    default:
      return { view: null };
  }
}

/** The path (with its query string) that shows a route. */
export function routePath(r: Route): string {
  switch (r.view) {
    case null:
      return '/';
    case 'pm':
      return r.pm === 'current' ? '/pm' : `/pm/${enc(r.pm)}`;
    case 'sessions':
      return r.workspaceId ? `/sessions/${enc(r.workspaceId)}${r.tabId ? `/${enc(r.tabId)}` : ''}` : '/sessions';
    case 'inbox':
      return r.itemId ? `/inbox/${enc(r.itemId)}` : '/inbox';
    case 'tickets':
      return `/tickets${r.ticket ? `/${enc(r.ticket)}` : ''}${r.board ? '?view=board' : ''}`;
    case 'workflows': {
      const t = r.target;
      if (!t) return '/workflows';
      if (t.kind === 'template') return `/workflows/templates/${enc(t.id)}`;
      if (t.runId) return `/workflows/${enc(t.id)}/runs/${enc(t.runId)}`;
      return `/workflows/${enc(t.id)}${t.tab ? `/${t.tab}` : ''}`;
    }
    case 'personas':
      return r.personaId ? `/personas/${enc(r.personaId)}` : '/personas';
    case 'files': {
      const q = new URLSearchParams();
      if (r.project) q.set('project', r.project);
      if (r.worktree) q.set('worktree', r.worktree);
      const search = q.toString();
      return `/files${r.path ? `/${r.path.split('/').map(enc).join('/')}` : ''}${search ? `?${search}` : ''}`;
    }
    case 'settings': {
      const p = r.page;
      switch (p.kind) {
        case 'host': return p.section ? `/settings/host/${enc(p.section)}` : '/settings/host';
        case 'connections': return '/settings/connections';
        case 'notifications': return '/settings/notifications';
        case 'project': return `/settings/projects/${enc(p.key)}${p.section ? `/${enc(p.section)}` : ''}`;
      }
    }
  }
}

/** A view's own path, showing nothing in particular in it. */
export function viewRoute(view: ViewId): Route {
  switch (view) {
    case 'pm': return { view, pm: 'current' };
    case 'sessions': return { view, workspaceId: null, tabId: null };
    case 'inbox': return { view, itemId: null };
    case 'tickets': return { view, board: false, ticket: null };
    case 'workflows': return { view, target: null };
    case 'personas': return { view, personaId: null };
    case 'files': return { view, path: null, project: null, worktree: null };
    case 'settings': return { view, page: { kind: 'host', section: null } };
  }
}

/** A Settings section's name in a path: its heading in lower case, words joined by `-` ("Danger zone" → `danger-zone`). */
export const sectionSlug = (label: string) => label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/** Reads a location (path and query string, as `useLocation` gives it) as a route. */
export function parseLocation(loc: string): Route {
  const i = loc.indexOf('?');
  return i < 0 ? parseRoute(loc) : parseRoute(loc.slice(0, i), loc.slice(i));
}
