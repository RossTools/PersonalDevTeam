import { useEffect, useState, type ReactNode } from 'react';
import { Bot, Cog, User } from 'lucide-react';
import { cn } from './lib/utils';

export interface TimelineItem {
  id: string;
  /** Who did it: `user` (you), `system` (PDT itself), `agent` (an agent through the API), or a persona name. */
  actor: 'user' | 'system' | 'agent' | (string & {});
  /** Shown in place of the actor's name (an agent's persona and ticket, linking to its session). */
  actorLabel?: ReactNode;
  /** Overrides the actor's default icon (a lucide icon at 12px). */
  icon?: ReactNode;
  /** When it happened (ms since the epoch). */
  time: number;
  /** One line: "Moved to Ready", "Comment". */
  title: ReactNode;
  /** Optional body under the title (a comment's Markdown, a diff summary). */
  body?: ReactNode;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "5m ago", "2h ago", "3d ago", then a date ("12 Mar", or "12 Mar 2025" in another year). */
export function relativeTime(time: number, now: number = Date.now()): string {
  const d = Math.max(0, now - time);
  if (d < MINUTE) return 'just now';
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m ago`;
  if (d < DAY) return `${Math.floor(d / HOUR)}h ago`;
  if (d < 7 * DAY) return `${Math.floor(d / DAY)}d ago`;
  const date = new Date(time);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return date.toLocaleDateString(undefined, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) });
}

/** The full local date and time, for tooltips. */
export function absoluteTime(time: number): string {
  return new Date(time).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** The current time, refreshed every minute, so relative times stay right on a page left open. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), MINUTE);
    return () => clearInterval(t);
  }, []);
  return now;
}

/** A time shown relatively ("2h ago") with the absolute time in its tooltip. */
export function RelativeTime({ time, now, className }: { time: number; now?: number; className?: string }) {
  const live = useNow();
  return (
    <time dateTime={new Date(time).toISOString()} title={absoluteTime(time)} className={cn('tabular-nums', className)}>
      {relativeTime(time, now ?? live)}
    </time>
  );
}

const NAMES: Record<string, string> = { user: 'You', system: 'PDT', agent: 'Agent' };
const actorName = (actor: string) => NAMES[actor] ?? actor;

function ActorIcon({ actor }: { actor: string }) {
  if (actor === 'user') return <User aria-hidden />;
  if (actor === 'system') return <Cog aria-hidden />;
  if (actor === 'agent') return <Bot aria-hidden />;
  return (
    <span aria-hidden className="text-2xs leading-none font-semibold uppercase">
      {actor.slice(0, 1)}
    </span>
  );
}

export interface TimelineProps {
  items: TimelineItem[];
  /** Shown when there are no items. */
  empty?: ReactNode;
  /** Fixes "now" (tests, screenshots); defaults to the clock. */
  now?: number;
  'aria-label'?: string;
  className?: string;
}

/** A compact activity list (oldest first, as given): actor icon on a rail, title, actor and relative time, body. */
export function Timeline({ items, empty = 'No activity yet.', now, 'aria-label': label = 'Activity', className }: TimelineProps) {
  if (items.length === 0) {
    return <p className={cn('py-2 text-xs text-muted-foreground', className)}>{empty}</p>;
  }
  return (
    <ol aria-label={label} className={cn('flex flex-col text-sm', className)}>
      {items.map((item, i) => (
        <li key={item.id} className="relative grid grid-cols-[1.25rem_minmax(0,1fr)] gap-x-2 pb-3 last:pb-0">
          {i < items.length - 1 && (
            <span aria-hidden className="absolute top-5 bottom-0 left-2.5 w-px -translate-x-1/2 bg-border" />
          )}
          <span className="relative flex size-5 items-center justify-center rounded-full border border-border bg-card text-muted-foreground [&>svg]:size-3">
            {item.icon ?? <ActorIcon actor={item.actor} />}
          </span>
          <div className="flex min-w-0 flex-col gap-1">
            <div className="flex min-h-5 flex-wrap items-baseline gap-x-1.5 text-xs">
              <span className="min-w-0 font-medium text-foreground">{item.title}</span>
              <span className="text-meta text-muted-foreground">
                {item.actorLabel ?? actorName(item.actor)} · <RelativeTime time={item.time} now={now} />
              </span>
            </div>
            {item.body != null && <div className="min-w-0 text-sm">{item.body}</div>}
          </div>
        </li>
      ))}
    </ol>
  );
}
