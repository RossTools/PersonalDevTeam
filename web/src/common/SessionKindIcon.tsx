import { Code, Sparkles, SquareTerminal, type LucideIcon } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import type { Session } from '../api/gen/Session';

export type SessionKindName = 'claude' | 'codex' | 'shell';

/** One icon per thing a session runs (tabs, agent rows, the palette, the new-tab chooser). */
export const KIND_ICON: Record<SessionKindName, LucideIcon> = { claude: Sparkles, codex: Code, shell: SquareTerminal };
export const KIND_LABEL: Record<SessionKindName, string> = { claude: 'Claude', codex: 'Codex', shell: 'Shell' };

type KindOf = Pick<Session, 'kind' | 'agent'>;

/** What a session runs. Session titles don't name it (PDT-20); this icon and label do. */
export function sessionKindName(s: KindOf): SessionKindName {
  if (s.kind === 'shell') return 'shell';
  return s.agent === 'codex' ? 'codex' : 'claude';
}

export const sessionKindLabel = (s: KindOf) => KIND_LABEL[sessionKindName(s)];

/** The session's kind as a small muted icon, labelled for screen readers and on hover. Nothing without a session. */
export function SessionKindIcon({ session, className }: { session: KindOf | undefined; className?: string }) {
  if (!session) return null;
  const name = sessionKindName(session);
  const Icon = KIND_ICON[name];
  return (
    <span role="img" aria-label={KIND_LABEL[name]} title={KIND_LABEL[name]} className={cn('inline-flex shrink-0 text-muted-foreground', className)}>
      <Icon aria-hidden className="size-3.5" />
    </span>
  );
}
