import { useEffect, useMemo, useRef } from 'react';
import type { InboxItem } from '../api/gen/InboxItem';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import { itemState } from '../common/AgentState';
import { useInbox } from '../inbox/useInbox';
import { isUnread, usePm } from '../pm/usePm';
import type { ItemInfo } from '../workspaces/useWorkspaces';

/**
 * The most important thing waiting on the user (PDT-207), most important first: `needs_you` when a session needs them
 * (an open permission or question request), `inbox` when the Inbox has items, `idle` when an agent or run has finished
 * and they haven't seen it (or a PM reply is unread), else `none`.
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
  pm: PmSessionInfo[];
  pmShown: string | null;
}

/**
 * The favicon's level and the title's count. A session that needs the user is `needs_you` whether its Inbox item or
 * its own activity says so; the count is the Inbox's alone.
 */
export function attentionOf({ inbox, items, pm, pmShown }: AttentionSources): Attention {
  const open = inbox.filter((i) => i.status === 'open');
  const states = items.map(itemState);
  const level: AttentionLevel =
    open.some((i) => i.kind === 'session') || states.some((s) => s.label === 'Needs you') ? 'needs_you'
    : open.length > 0 ? 'inbox'
    : states.some((s) => s.unseen) || pm.some((p) => isUnread(p, pmShown)) ? 'idle'
    : 'none';
  return { level, count: open.length };
}

/**
 * [attentionOf] over the queries the rail already reads (the stream refreshes the Inbox's; no polling of its own).
 * `items` are the workspaces' session and run infos, `pmShown` the PM conversation on screen.
 */
export function useAttention(items: ItemInfo[], pmShown: string | null): Attention {
  const inbox = useInbox().data?.items;
  const pm = usePm().data?.sessions;
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
 * `svg` (the plain favicon, a 32×32 viewBox) with a dot in its top right corner, ringed in the page's surface colour
 * so it stands off the icon. The colours come from tokens.css.
 */
export function badgedSvg(svg: string, level: Exclude<AttentionLevel, 'none'>): string {
  const dot = `<circle cx="25" cy="7" r="8" fill="${token('--pdt-surface')}"/><circle cx="25" cy="7" r="6" fill="${token(DOT_TOKEN[level])}"/>`;
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
