import { useEffect, useMemo, useRef } from 'react';
import type { InboxItem } from '../api/gen/InboxItem';
import { itemState } from '../common/AgentState';
import { useInbox } from '../inbox/useInbox';
import { pmUnread, usePmSummary, type PmSummary } from '../pm/usePm';
import type { ItemInfo } from '../workspaces/useWorkspaces';

/**
 * The most important thing waiting on the user (PDT-207), most important first: `needs_you` when a session needs them
 * (an open permission or question request), `inbox` when the Inbox has items, `idle` when something they started (a
 * session or run, not a workflow's step session: PDT-262) has finished and they haven't seen it (or a PM reply is
 * unread), else `none`.
 */
export type AttentionLevel = 'none' | 'idle' | 'inbox' | 'needs_you';

export interface Attention {
  level: AttentionLevel;
  /** The Inbox's open items, and nothing else: the page title's `(n)`. */
  count: number;
}

export interface AttentionSources {
  /** The Inbox's open items (sessions waiting included). */
  inbox: InboxItem[];
  /** Every project's sessions and runs (the rail's and tree's dots). */
  items: ItemInfo[];
  /** The PM's conversations, and the one on screen (it reads as seen). */
  pm: PmSummary[];
  pmShown: string | null;
}

/**
 * The favicon's level and the title's count. A session that needs the user is `needs_you` whether its Inbox item or
 * its own activity says so; the count is the Inbox's alone.
 */
export function attentionOf({ inbox, items, pm, pmShown }: AttentionSources): Attention {
  const open = inbox.filter((i) => i.status === 'open');
  const states = items.map(itemState);
  // Workflow step sessions (Coder, Reviewer, QA…) finish all the time: news only when they need the user.
  const news = items.filter((i) => !isStepSession(i)).map(itemState);
  const level: AttentionLevel =
    open.some((i) => i.kind === 'session') || states.some((s) => s.label === 'Needs you') ? 'needs_you'
    : open.length > 0 ? 'inbox'
    : news.some((s) => s.unseen) || pmUnread(pm, pmShown) ? 'idle'
    : 'none';
  return { level, count: open.length };
}

/** A session a workflow's agent step started, rather than the user. */
const isStepSession = (i: ItemInfo) => !!i.session?.persona?.stepId;

/**
 * [attentionOf] over the queries the rail already reads (the stream refreshes the Inbox's; no polling of its own).
 * `items` are the workspaces' session and run infos, `pmShown` the PM conversation on screen.
 */
export function useAttention(items: ItemInfo[], pmShown: string | null): Attention {
  const inbox = useInbox().data?.items;
  const pm = usePmSummary().data?.conversations;
  const { level, count } = useMemo(
    () => attentionOf({ inbox: inbox ?? [], items, pm: pm ?? [], pmShown }),
    [inbox, items, pm, pmShown],
  );
  return useMemo(() => ({ level, count }), [level, count]);
}

/** The token each level's dot is drawn in: red when a session needs the user, amber for the Inbox, the accent for news. */
export const DOT_TOKEN: Record<Exclude<AttentionLevel, 'none'>, string> = {
  needs_you: '--pdt-failed',
  inbox: '--pdt-waiting',
  idle: '--pdt-accent',
};

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/**
 * `svg` (the plain favicon, a square viewBox) with a dot in its top right corner, ringed in the page's surface colour
 * so it stands off the icon. The dot is sized for a 32-wide viewBox and scaled to the icon's own (32 without one). The
 * colours come from tokens.css.
 */
export function badgedSvg(svg: string, level: Exclude<AttentionLevel, 'none'>): string {
  const width = Number(/viewBox="[\d.-]+ [\d.-]+ ([\d.]+)/.exec(svg)?.[1]) || 32;
  const [cx, cy, ring, r] = [25, 7, 8, 6].map((n) => +(n * width / 32).toFixed(3));
  const at = `cx="${cx}" cy="${cy}"`;
  const dot = `<circle ${at} r="${ring}" fill="${token('--pdt-surface')}"/><circle ${at} r="${r}" fill="${token(DOT_TOKEN[level])}"/>`;
  return svg.replace(/<\/svg>\s*$/, `${dot}</svg>`);
}

const svgUrl = (svg: string) => `data:image/svg+xml,${encodeURIComponent(svg)}`;

/**
 * Swaps the page's `<link rel="icon">` for the plain icon with a dot while `level` isn't `none`, and back. The plain
 * icon is fetched once from the link's own href; if that fails the icon stays plain.
 */
export function useFaviconBadge(level: AttentionLevel) {
  const plain = useRef<{ href: string; svg: Promise<string | null> } | null>(null);
  useEffect(() => {
    const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
    if (!link) return;
    if (!plain.current) {
      const href = link.getAttribute('href') ?? '';
      plain.current = {
        href,
        svg: fetch(href).then((r) => (r.ok ? r.text() : null)).catch(() => null),
      };
    }
    const { href, svg } = plain.current;
    if (level === 'none') {
      link.setAttribute('href', href);
      return;
    }
    let live = true;
    void svg.then((text) => {
      if (live && text) link.setAttribute('href', svgUrl(badgedSvg(text, level)));
    });
    return () => { live = false; };
  }, [level]);
  useEffect(() => () => {
    if (plain.current) document.querySelector('link[rel="icon"]')?.setAttribute('href', plain.current.href);
  }, []);
}
