import { createContext } from 'react';
import type { BadgeTone } from '@ui';
import type { ForgeLinks } from '../api/gen/ForgeLinks';

/**
 * What a conversation's Markdown knows about where it is (PDT-382). With it, ticket keys show as pills with a hover
 * card, `#12` and the forge's PR links as PR pills, paths of the session's files as links to the file viewer, and
 * commit SHAs in muted mono. Without it (the default: ticket descriptions, notes), ticket keys stay plain links.
 */
export interface MarkdownRefs {
  /** The session whose folder file paths are looked up in. */
  sessionId: string | null;
  /** The project's key and checkout, for the file viewer's path. */
  projectKey: string | null;
  worktreeId: string | null;
  /** The project's repository on its forge (PR numbers link there). */
  forge: ForgeLinks | null;
}

export const MarkdownRefsContext = createContext<MarkdownRefs | null>(null);

const trimSlash = (s: string) => s.replace(/\/+$/, '');

/** A change request's page on the forge: GitHub's `/pull/12`, GitLab's `/-/merge_requests/12`; null for others. */
export function prUrl(forge: ForgeLinks | null, n: number): string | null {
  if (!forge) return null;
  const repo = trimSlash(forge.repoUrl);
  switch (forge.kind) {
    case 'github':
      return `${repo}/pull/${n}`;
    case 'gitlab':
      return `${repo}/-/merge_requests/${n}`;
    default:
      return null;
  }
}

/** The PR number a link to the forge's change request names, or null. */
export function prNumberOf(forge: ForgeLinks | null, href: string | undefined): number | null {
  if (!forge || !href) return null;
  const at = prUrl(forge, 0);
  if (!at) return null;
  const base = at.slice(0, -1);
  if (!href.startsWith(base)) return null;
  const m = /^(\d+)(?:[/?#].*)?$/.exec(href.slice(base.length));
  return m ? Number(m[1]) : null;
}

/** A commit SHA as people paste them: 7 to 40 hex digits, with a digit and a letter (not a word, not a number). */
export const isSha = (s: string) => /^[0-9a-f]{7,40}$/.test(s) && /[a-f]/.test(s) && /\d/.test(s);

/**
 * A file path as an agent writes one (`src/app.ts`, `README.md`, `web/src/main.tsx:12`): its path without the line,
 * or null when the text isn't one (a URL, a sentence, a version number).
 */
export function filePathOf(text: string): string | null {
  const t = text.trim().replace(/^\.\//, '');
  if (!t || /\s/.test(t) || /^[a-z][a-z0-9+.-]*:/i.test(t) || t.startsWith('/') || t.startsWith('~')) return null;
  const path = t.replace(/:\d+(?::\d+)?(?:-\d+)?$/, '');
  if (path.includes('..') || path.endsWith('/')) return null;
  // A file: a name with an extension of letters, in folders or not.
  return /^(?:[\w@.+-]+\/)*[\w@+-][\w@.+-]*\.[A-Za-z][A-Za-z0-9]{0,9}$/.test(path) ? path : null;
}

/** The file viewer's path for a file of the project's checkout. */
export function fileHref(refs: MarkdownRefs, path: string): string {
  const q = new URLSearchParams();
  if (refs.projectKey) q.set('project', refs.projectKey);
  if (refs.worktreeId) q.set('worktree', refs.worktreeId);
  const search = q.toString();
  return `/files/${path.split('/').map(encodeURIComponent).join('/')}${search ? `?${search}` : ''}`;
}

/**
 * The colour of a status word in a table cell ("CI failed", "ready to accept", "Coder working"), or null for text
 * that isn't one. Colour means state: green for passed or working, amber for waiting on someone, red for failed.
 */
export function statusTone(text: string): BadgeTone | null {
  const t = text.trim().toLowerCase();
  if (!t || t.length > 32) return null;
  if (/^(ci )?(failed|failing)$|^(build|checks?) failed$|^stalled$|^\w+ stalled$/.test(t)) return 'failed';
  if (/^(ci )?(passed|passing|green)$|^merged$|^done$|^checks? passed$|^working$|^\w+ working$/.test(t)) return 'running';
  if (/^(in review|ready to accept|needs you|waiting|in the merge queue|queued|ci running|ci queued)$|^\w+ asks you$/.test(t)) return 'waiting';
  if (/^blocked( by [\w-]+)?$|^won'?t do$|^draft$|^to do$/.test(t)) return 'neutral';
  return null;
}
