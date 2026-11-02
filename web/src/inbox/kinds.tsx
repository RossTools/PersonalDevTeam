// Each Inbox kind's icon and colour (PDT-361): the list's icons and the kind chip over an item.
import type { ComponentType } from 'react';
import {
  CircleAlert, CircleCheck, CircleHelp, CircleX, Clock, FileCheck, FilePlus, GitBranch, Hand, HardDrive, KeyRound, LogIn, MessageSquare,
  Plug, RotateCw, ShieldQuestion, SquareTerminal, type LucideProps,
} from 'lucide-react';
import { Badge, type BadgeTone } from '@ui';
import { cn } from '@ui/lib/utils';
import type { InboxItem } from '../api/gen/InboxItem';
import type { InboxKind } from '../api/gen/InboxKind';
import { itemLabel } from './model';

type Tone = Exclude<BadgeTone, 'neutral'>;

const KINDS: Record<InboxKind, { icon: ComponentType<LucideProps>; tone: Tone }> = {
  question: { icon: CircleHelp, tone: 'accent' },
  secret: { icon: KeyRound, tone: 'accent' },
  approval: { icon: SquareTerminal, tone: 'waiting' },
  confirm: { icon: ShieldQuestion, tone: 'waiting' },
  stalled: { icon: CircleAlert, tone: 'failed' },
  loop_limit: { icon: RotateCw, tone: 'failed' },
  timed_out: { icon: Clock, tone: 'failed' },
  session: { icon: MessageSquare, tone: 'waiting' },
  notice: { icon: CircleX, tone: 'failed' },
  connect: { icon: Plug, tone: 'waiting' },
  mcp_sign_in: { icon: LogIn, tone: 'waiting' },
  storage: { icon: HardDrive, tone: 'failed' },
  low_disk: { icon: HardDrive, tone: 'waiting' },
  admit: { icon: FileCheck, tone: 'accent' },
  proposal: { icon: GitBranch, tone: 'accent' },
  ticket_proposal: { icon: FilePlus, tone: 'accent' },
  gate: { icon: CircleCheck, tone: 'accent' },
  manual: { icon: Hand, tone: 'accent' },
};

/** An item's icon and colour: a gate under Ready to accept is green, a disk nearly full red. */
export function kindStyle(item: InboxItem): { icon: ComponentType<LucideProps>; tone: Tone } {
  const k = KINDS[item.kind];
  if (item.kind === 'gate' && item.group === 'accept') return { ...k, tone: 'running' };
  if (item.kind === 'low_disk' && item.body.critical === true) return { ...k, tone: 'failed' };
  return k;
}

const soft: Record<Tone, string> = {
  accent: 'bg-accent text-link',
  running: 'bg-running-soft text-running',
  waiting: 'bg-waiting-soft text-waiting',
  failed: 'bg-failed-soft text-failed',
};

/** The kind's icon in its colour, on a small square (a list row's leading icon). */
export function KindIcon({ item, className }: { item: InboxItem; className?: string }) {
  const { icon: Icon, tone } = kindStyle(item);
  return (
    <span aria-hidden className={cn('grid size-4.5 shrink-0 place-items-center rounded-sm', soft[tone], className)}>
      <Icon className="size-3" />
    </span>
  );
}

/** The kind as a chip in its colour: "? Question", "✓ Accept". */
export function KindChip({ item }: { item: InboxItem }) {
  const { icon: Icon, tone } = kindStyle(item);
  return (
    <Badge tone={tone} className="rounded-full font-semibold">
      <Icon aria-hidden />
      {itemLabel(item)}
    </Badge>
  );
}
