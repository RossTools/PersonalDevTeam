import { useEffect, useMemo, useRef } from 'react';
import type { InboxItem } from '../api/gen/InboxItem';
import type { InboxKind } from '../api/gen/InboxKind';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import { agentState } from '../common/AgentState';
import { useInbox } from '../inbox/useInbox';
import { isUnread, usePm } from '../pm/usePm';
import type { ItemInfo } from '../workspaces/useWorkspaces';
import { useUpdates } from './queries';

/**
 * How much wants the user (PDT-168): `blocking` when something waits on them (an Inbox item they must answer, a
 * session that needs them), `unseen` when there is only news (an unseen session, an unread PM reply or Update, an
 * unheard notice), else `none`.
 */
export type AttentionLevel = 'none' | 'unseen' | 'blocking';

export interface Attention {
  level: AttentionLevel;
  /** Distinct things wanting the user, blocking or not: the page title's `(n)`. */
  count: number;
  blocking: number;
}

/** Inbox kinds that are news rather than a wait: everything else blocks a step or a session on the user. */
const NEWS: ReadonlySet<InboxKind> = new Set<InboxKind>(['notice']);

export interface AttentionSources {
  /** The Inbox's open items (sessions waiting included). */
  inbox: InboxItem[];
  /** Unread Updates (the Inbox rail's dot). */
  unreadUpdates: number;
  /** Every project's sessions and runs (the rail's and tree's dots). */
  items: ItemInfo[];
  /** The PM's conversations, and the one on screen (it reads as seen). */
  pm: PmSessionInfo[];
  pmShown: string | null;
}

/**
 * Combines every kind of notification the app tracks into one level and count. A session counts once, however many
 * sources name it (its Inbox item and its own `needs_you`), at its most urgent.
 */
export function attentionOf({ inbox, unreadUpdates, items, pm, pmShown }: AttentionSources): Attention {
  const blocking = new Set<string>();
  const news = new Set<string>();
  for (const i of inbox) {
    if (i.status !== 'open') continue;
    const key = i.sessionId && i.kind === 'session' ? `session:${i.sessionId}` : `inbox:${i.id}`;
    (NEWS.has(i.kind) ? news : blocking).add(key);
  }
  for (const info of items) {
    const s = info.session;
    if (!s) continue;
    const state = agentState({ kind: s.kind, status: s.status, activity: s.activity, unseen: s.unseen });
    if (state.label === 'Needs you') blocking.add(`session:${s.id}`);
    else if (state.unseen) news.add(`session:${s.id}`);
  }
  for (const p of pm) if (isUnread(p, pmShown)) news.add(`session:${p.session.id}`);
  for (const k of blocking) news.delete(k);
  const count = blocking.size + news.size + Math.max(0, unreadUpdates);
  const level: AttentionLevel = blocking.size > 0 ? 'blocking' : count > 0 ? 'unseen' : 'none';
  return { level, count, blocking: blocking.size };
}

/**
 * [attentionOf] over the queries the rail already reads (the stream refreshes the Inbox's and Updates'; no polling
 * of its own). `items` are the workspaces' session and run infos, `pmShown` the PM conversation on screen.
 */
export function useAttention(items: ItemInfo[], pmShown: string | null): Attention {
  const inbox = useInbox().data?.items;
  const unreadUpdates = useUpdates().data?.unreadUpdates ?? 0;
  const pm = usePm().data?.sessions;
  const { level, count, blocking } = useMemo(
    () => attentionOf({ inbox: inbox ?? [], unreadUpdates, items, pm: pm ?? [], pmShown }),
    [inbox, unreadUpdates, items, pm, pmShown],
  );
  return useMemo(() => ({ level, count, blocking }), [level, count, blocking]);
}

/** The token each level's dot is drawn in: amber for a wait, the accent for news. */
const DOT_TOKEN: Record<Exclude<AttentionLevel, 'none'>, string> = { blocking: '--pdt-waiting', unseen: '--pdt-accent' };

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
