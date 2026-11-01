// The New workflow gallery's model (PDT-229): its categories, search, and what each category shows. Pure.
import type { GalleryTemplate } from '../api/gen/GalleryTemplate';
import type { TemplateCategory } from '../api/gen/TemplateCategory';

/** The blank template: the gallery's "Start from scratch". */
export const BLANK = 'builtin:blank';

/** The categories, in the gallery's order, with their labels. */
export const CATEGORIES: { id: TemplateCategory; label: string }[] = [
  { id: 'ciPr', label: 'CI & pull requests' },
  { id: 'tickets', label: 'Tickets' },
  { id: 'issues', label: 'Issues' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'releases', label: 'Releases' },
  { id: 'pm', label: 'PM' },
  { id: 'documents', label: 'Documents' },
  { id: 'other', label: 'Other' },
];

/** What the left column picks: every template, one category, or PDT's built-in workflows. */
export type GalleryFilter = 'all' | 'builtin' | TemplateCategory;

/** Whether the template's name, description or plugin has `q` in it (any case). */
export function matches(t: GalleryTemplate, q: string): boolean {
  const needle = q.trim().toLowerCase();
  if (!needle) return true;
  return [t.name, t.description, t.plugin?.label ?? ''].some((s) => s.toLowerCase().includes(needle));
}

/** A template a plugin that is off is needed for: shown faded, and it can't be created. */
export const pluginOff = (t: GalleryTemplate) => t.plugin !== null && !t.plugin.on;

/** The trigger, as a card's pill shows it. */
export function triggerLabel(t: GalleryTemplate): string {
  const [first, ...rest] = t.triggers;
  if (!first) return 'No trigger';
  return rest.length ? `${first} +${rest.length}` : first;
}

export interface Gallery {
  /** Start from scratch, when it matches (first, on its own, under All). */
  blank: GalleryTemplate | null;
  /** The templates of each category that has any matching, in order. */
  groups: { id: TemplateCategory; label: string; items: GalleryTemplate[] }[];
  /** PDT's built-in workflows that match (their own group, at the end). */
  builtins: GalleryTemplate[];
  /** The left column: All, the categories that have templates at all (empty ones are hidden), then Built-in; each
   * with how many match. */
  counts: { all: number; categories: { id: TemplateCategory; label: string; count: number }[]; builtin: number };
  /** Nothing matches the search. */
  empty: boolean;
}

/** The gallery for `list` with the search `q`, under the filter `filter`. */
export function gallery(list: GalleryTemplate[], q: string, filter: GalleryFilter): Gallery {
  const visible = list.filter((t) => matches(t, q));
  const plain = visible.filter((t) => !t.builtinWorkflow);
  const builtins = visible.filter((t) => t.builtinWorkflow);
  const blank = plain.find((t) => t.id === BLANK) ?? null;
  const inCategory = (c: TemplateCategory, from: GalleryTemplate[]) => from.filter((t) => t.category === c && t.id !== BLANK);
  const present = CATEGORIES.filter((c) => inCategory(c.id, list.filter((t) => !t.builtinWorkflow)).length > 0);
  const counts = {
    all: plain.length,
    categories: present.map((c) => ({ ...c, count: inCategory(c.id, plain).length })),
    builtin: builtins.length,
  };
  const groups = CATEGORIES.map((c) => ({ ...c, items: inCategory(c.id, plain) })).filter((g) => g.items.length > 0);
  const empty = visible.length === 0;
  if (filter === 'all') return { blank, groups, builtins, counts, empty };
  if (filter === 'builtin') return { blank: null, groups: [], builtins, counts, empty };
  return { blank: null, groups: groups.filter((g) => g.id === filter), builtins: [], counts, empty };
}
