// The app's URL paths (PDT-111): what each one shows, and the path for a screen. Pure: router.ts does the history.
import type { PmSelection } from '../pm/usePm';
import type { WorkflowTab } from '../workflows/WorkflowPage';
import type { WorkflowTarget } from '../workflows/WorkflowsList';
import type { ViewId } from './views';

/**
 * A Settings page: the index (`/settings`: this host on desktop, the list of pages on a phone), this host (and a
 * section of it), connections, notifications, or a project by key.
 */
export type SettingsPage =
  | { kind: 'index' }
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
  /**
   * Notes (PDT-143): the user's own (`memory: false`, `/notes`) or the PM's memory (`/notes/pm`), and one note by id
   * (`/notes/<id>`) or a new one (`note: 'new'`, `/notes/new`). A note's path doesn't say whose it is.
   */
  | { view: 'notes'; memory: boolean; note: string | null }
  | { view: 'workflows'; target: WorkflowTarget }
  | { view: 'personas'; personaId: string | null }
  /**
   * A file in a checkout, or a folder (its path ends in `/`: the phone's drill-down, PDT-120); `project` (a key) and
   * `worktree` say which checkout when the user picked one.
   */
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
      // The notes' old paths (ticket descriptions link to `/pm/notes/<id>`): the Notes view's.
      if (a === 'notes') return b ? { view: 'notes', memory: false, note: b } : { view: 'notes', memory: true, note: null };
      return { view: 'pm', pm: a ?? 'current' };
    case 'notes':
      return a === 'pm' ? { view: 'notes', memory: true, note: null } : { view: 'notes', memory: false, note: a };
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
      return {
        view: 'files',
        path: seg.length > 1 ? `${seg.slice(1).join('/')}${pathname.endsWith('/') ? '/' : ''}` : null,
        project: q.get('project'),
        worktree: q.get('worktree'),
      };
    case 'settings':
      if (a === 'connections') return { view: 'settings', page: { kind: 'connections' } };
      if (a === 'notifications') return { view: 'settings', page: { kind: 'notifications' } };
      if (a === 'projects' && b) return { view: 'settings', page: { kind: 'project', key: b, section: c } };
      if (a === 'host') return { view: 'settings', page: { kind: 'host', section: b } };
      return { view: 'settings', page: { kind: 'index' } };
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
    case 'notes':
      return r.note ? `/notes/${enc(r.note)}` : r.memory ? '/notes/pm' : '/notes';
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
        case 'index': return '/settings';
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
    case 'notes': return { view, memory: false, note: null };
    case 'workflows': return { view, target: null };
    case 'personas': return { view, personaId: null };
    case 'files': return { view, path: null, project: null, worktree: null };
    case 'settings': return { view, page: { kind: 'index' } };
  }
}

/**
 * A screen inside a view (an item, a page, a tab), as opposed to the view's own top-level screen. On a phone a detail
 * screen has ← (Back) in its top bar instead of ☰.
 */
export function isDetail(r: Route): boolean {
  switch (r.view) {
    case null: return false;
    case 'pm': return r.pm !== 'current';
    case 'sessions': return r.workspaceId !== null;
    case 'inbox': return r.itemId !== null;
    case 'tickets': return r.ticket !== null;
    case 'notes': return r.note !== null;
    case 'workflows': return r.target !== null;
    case 'personas': return r.personaId !== null;
    case 'files': return r.path !== null;
    case 'settings': return r.page.kind !== 'index';
  }
}

/** Where ← goes from a detail screen when there is no history to go back through (opened from a link): its view. */
export function upRoute(r: Route): Route {
  if (r.view === null) return r;
  if (r.view === 'tickets') return { ...r, ticket: null };
  // An earlier session goes up to the list of them.
  if (r.view === 'pm' && r.pm !== 'current' && r.pm !== 'earlier') return { view: 'pm', pm: 'earlier' };
  // A run goes up to the workflow's runs.
  if (r.view === 'workflows' && r.target?.kind === 'workflow' && r.target.runId) return { view: 'workflows', target: { kind: 'workflow', id: r.target.id, tab: 'runs' } };
  // A file or folder goes up to the folder it is in.
  if (r.view === 'files') return { ...r, path: parentFolder(r.path) };
  // A Settings section (a phone's screen of its own) goes up to its page.
  if (r.view === 'settings' && (r.page.kind === 'host' || r.page.kind === 'project') && r.page.section !== null) {
    return { view: 'settings', page: { ...r.page, section: null } };
  }
  return viewRoute(r.view);
}

/** The folder a file or folder is in, as a folder path (`a/b.md` → `a/`); null at the checkout's top. */
export function parentFolder(path: string | null): string | null {
  const parts = (path ?? '').split('/').filter(Boolean);
  return parts.length > 1 ? `${parts.slice(0, -1).join('/')}/` : null;
}

/** A Settings section's name in a path: its heading in lower case, words joined by `-` ("Danger zone" → `danger-zone`). */
export const sectionSlug = (label: string) => label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/** Reads a location (path and query string, as `useLocation` gives it) as a route. */
export function parseLocation(loc: string): Route {
  const i = loc.indexOf('?');
  return i < 0 ? parseRoute(loc) : parseRoute(loc.slice(0, i), loc.slice(i));
}
