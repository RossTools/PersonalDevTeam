// Settings → Plugins (PDT-228): the catalog's filters (search, tabs, categories), the statuses and the Cards/List
// choice. Pure, apart from the stored choice. A plugin is an integration (crates/pdt-api/src/integrations/); the UI
// calls them plugins.
import type { BadgeTone } from '@ui';
import type { Connection } from '../api/gen/Connection';
import type { IntegrationInfo } from '../api/gen/IntegrationInfo';
import type { PluginCategory } from '../api/gen/PluginCategory';
import type { PluginStatus } from '../api/gen/PluginStatus';

export type Plugin = IntegrationInfo;

export const pluginsKey = ['integrations'] as const;

/** The catalog's groups, in order. */
export const CATEGORIES: readonly PluginCategory[] = ['dev', 'chat', 'media', 'general'];

export const CATEGORY_LABEL: Record<PluginCategory, string> = { dev: 'Dev', chat: 'Chat', media: 'Media', general: 'General' };

export const STATUS_LABEL: Record<PluginStatus, string> = {
  on: 'On', off: 'Off', needs_setup: 'Needs setup', error: 'Error', not_built: 'Not in this build',
};

export const STATUS_TONE: Record<PluginStatus, BadgeTone> = {
  on: 'running', off: 'neutral', needs_setup: 'waiting', error: 'failed', not_built: 'neutral',
};

export type PluginTab = 'all' | 'on' | 'attention';

export interface PluginFilter {
  q: string;
  tab: PluginTab;
  /** null: every category. */
  category: PluginCategory | null;
}

export const NO_FILTER: PluginFilter = { q: '', tab: 'all', category: null };

/** On, whether or not it works yet. */
export const isOn = (p: Plugin) => p.status === 'on' || p.status === 'needs_setup' || p.status === 'error';

/** Needs the user: on without a connection, or one failing. */
export const needsAttention = (p: Plugin) => p.status === 'needs_setup' || p.status === 'error';

/** Whether a plugin passes the search and the tab (not the category: the chips count what these leave). */
function passes(p: Plugin, f: PluginFilter): boolean {
  const q = f.q.trim().toLowerCase();
  if (q && !`${p.label} ${p.description}`.toLowerCase().includes(q)) return false;
  if (f.tab === 'on') return isOn(p);
  if (f.tab === 'attention') return needsAttention(p);
  return true;
}

/** The plugins a filter shows. */
export const filterPlugins = (ps: Plugin[], f: PluginFilter) => ps.filter((p) => passes(p, f) && (f.category === null || p.category === f.category));

/** The shown plugins by category, in the catalog's order, leaving out empty groups. */
export function groupPlugins(ps: Plugin[]): [PluginCategory, Plugin[]][] {
  return CATEGORIES.map((c) => [c, ps.filter((p) => p.category === c)] as [PluginCategory, Plugin[]]).filter(([, g]) => g.length > 0);
}

/** How many plugins each category chip would show (and `all`), given the search and tab. */
export function categoryCounts(ps: Plugin[], f: PluginFilter): Record<PluginCategory | 'all', number> {
  const shown = ps.filter((p) => passes(p, f));
  const out = { all: shown.length } as Record<PluginCategory | 'all', number>;
  for (const c of CATEGORIES) out[c] = shown.filter((p) => p.category === c).length;
  return out;
}

/** The plugin a connection belongs to (by its kind). */
export const pluginOf = (ps: Plugin[], c: Connection) => ps.find((p) => p.connectionKinds.some((k) => k.id === c.kind));

/** A plugin's connections. */
export const connectionsOf = (p: Plugin, cs: Connection[]) => cs.filter((c) => p.connectionKinds.some((k) => k.id === c.kind));

/** The ids to enable when `id` is switched to `on`: the optional plugins that are on now, changed by that one. */
export const enabledWith = (ps: Plugin[], id: string, on: boolean) => ps.filter((p) => !p.core && (p.id === id ? on : p.enabled)).map((p) => p.id);

/** The ids of PDT's own to turn off when `id` is switched to `on` (PDT-251): those off now, changed by that one. */
export const offWith = (ps: Plugin[], id: string, on: boolean) => ps.filter((p) => p.canTurnOff && !(p.id === id ? on : p.enabled)).map((p) => p.id);

/** Why a plugin that is on can't be turned off now: projects use it as their forge (PDT-251). */
export function stuckOn(p: Plugin): string | null {
  if (!p.enabled || !p.forgeProjects.length) return null;
  const names = p.forgeProjects.map((x) => x.name).join(', ');
  return `${p.label} is the forge of ${names}. To turn it off, give ${p.forgeProjects.length > 1 ? 'them' : 'it'} another forge first (forge = "local" or "none" in .pdt/project.toml).`;
}

export type PluginLayout = 'cards' | 'list';

const LAYOUT_KEY = 'pdt.plugins.layout';

export function loadLayout(): PluginLayout {
  try {
    return localStorage.getItem(LAYOUT_KEY) === 'list' ? 'list' : 'cards';
  } catch {
    return 'cards';
  }
}

export function storeLayout(l: PluginLayout) {
  try {
    localStorage.setItem(LAYOUT_KEY, l);
  } catch {
    // Storage blocked (private window): the choice lasts for this page only.
  }
}
