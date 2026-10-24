// The Notes view's pure rules (PDT-143): previews, titles, sections and scopes. No React, no API.
import type { PmNote } from '../api/gen/PmNote';
import { routePath } from '../shell/route';

/** Whose notes a list shows: the user's own (`user`, "Mine") or the PM's memory (`note`). */
export type Whose = 'user' | 'note';

/** A list's scope: every note, the host-wide ones, or a project's (its id). */
export const ALL = 'all';
export const HOST_WIDE = 'host';

/** The longest title the API takes. */
export const TITLE_MAX = 200;

const DAY = 24 * 60 * 60 * 1000;

/** One line of Markdown as plain text: no heading, list, quote or task marks, emphasis, code ticks or link targets. */
function stripLine(line: string): string {
  return line
    .replace(/^\s*(?:#{1,6}\s+|>\s?)+/, '')
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/, '')
    .replace(/^\[[ xX]\]\s+/, '')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/(\*\*|__|~~|`)/g, '')
    .replace(/(^|[\s(])[*_](\S)/g, '$1$2')
    .replace(/(\S)[*_](?=$|[\s).,;:!?])/g, '$1')
    .trim();
}

/** Markdown as plain text, one entry per non-empty line (code fences and rules dropped). */
function plainLines(markdown: string): string[] {
  return markdown
    .split('\n')
    .filter((l) => !/^\s*(```|~~~|-{3,}\s*$|\*{3,}\s*$)/.test(l))
    .map(stripLine)
    .filter(Boolean);
}

/** A list item's preview: the body as plain text on one line (the list clamps it to two). */
export const previewOf = (body: string) => plainLines(body).join(' ');

/** The body's word count, as plain text. */
export const wordCount = (body: string) => plainLines(body).join(' ').split(/\s+/).filter(Boolean).length;

/**
 * The title a note is saved with: what was typed, else the body's first line as plain text, else "Untitled" (the API
 * rejects an empty title). At most 200 characters.
 */
export function titleFor(title: string, body: string): string {
  const t = title.trim() || plainLines(body)[0] || 'Untitled';
  return t.slice(0, TITLE_MAX).trim();
}

/** The notes in a scope: all, host-wide, or one project's. */
export function inScope(notes: PmNote[], scope: string): PmNote[] {
  if (scope === ALL) return notes;
  return notes.filter((n) => (scope === HOST_WIDE ? n.projectId === null : n.projectId === scope));
}

export interface NoteSection {
  id: 'pinned' | 'week' | 'earlier';
  label: string;
  notes: PmNote[];
}

/** The list's sections, each newest first: Pinned, Last 7 days, Earlier. Empty sections are left out. */
export function sectionsOf(notes: PmNote[], now: number = Date.now()): NoteSection[] {
  const byUpdated = [...notes].sort((a, b) => b.updatedAt - a.updatedAt);
  const sections: NoteSection[] = [
    { id: 'pinned', label: 'Pinned', notes: byUpdated.filter((n) => n.pinned) },
    { id: 'week', label: 'Last 7 days', notes: byUpdated.filter((n) => !n.pinned && now - n.updatedAt < 7 * DAY) },
    { id: 'earlier', label: 'Earlier', notes: byUpdated.filter((n) => !n.pinned && now - n.updatedAt >= 7 * DAY) },
  ];
  return sections.filter((s) => s.notes.length > 0);
}

/** "12 Oct", or "12 Oct 2025" in another year. */
export function shortDate(time: number, now: number = Date.now()): string {
  const d = new Date(time);
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) });
}

/** A ticket's description drafted from one of the user's notes: its body, then a link back to the note. */
export function ticketDescription(note: Pick<PmNote, 'id' | 'title' | 'body'>, origin = window.location.origin): string {
  const link = `${origin}${routePath({ view: 'notes', memory: false, note: note.id })}`;
  const back = `From the note [${note.title.replace(/[[\]]/g, '\\$&')}](${link}).`;
  return note.body.trim() ? `${note.body.trim()}\n\n${back}` : back;
}
