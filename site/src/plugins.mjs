// The website's Plugins pages (/plugins/, /plugins/media/, /plugins/<id>/): the plugins as Settings → Plugins lists
// them, from data/plugins.json, which pdtd's integrations tests write from the same code as the app (so it never
// drifts; CI fails when it is stale). Media plugins have their own page and are left out of the main one.
import data from './data/plugins.json' with { type: 'json' };

/** Every plugin, in the app's order: PDT's own first. */
export const PLUGINS = data.plugins;

/** The categories, in the app's order (web/src/plugins/model.ts). */
export const CATEGORIES = ['dev', 'documents', 'chat', 'media', 'general'];

export const CATEGORY_LABEL = { dev: 'Dev', documents: 'Documents', chat: 'Chat', media: 'Media', general: 'General' };

/** The categories on their own page, not on /plugins/. */
export const OWN_PAGE = ['media'];

/** The plugins /plugins/ lists (`media: false`) or /plugins/media/ does (`media: true`). */
export function pluginsOn(plugins, media) {
  return plugins.filter((p) => OWN_PAGE.includes(p.category) === media);
}

/** The shown plugins by category, in order, leaving out empty groups. */
export function groupPlugins(plugins) {
  return CATEGORIES.map((c) => [c, plugins.filter((p) => p.category === c)]).filter(([, g]) => g.length > 0);
}

/** Text with `code` spans as HTML (the steps' descriptions use them), everything else escaped. */
export function inlineCode(text) {
  const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return text
    .split('`')
    .map((s, i) => (i % 2 ? `<code>${esc(s)}</code>` : esc(s)))
    .join('');
}
