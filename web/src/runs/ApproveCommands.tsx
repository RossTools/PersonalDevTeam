import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, CodeCard, SectionLabel, Skeleton, type ConfirmOptions, type ConfirmResult } from '@ui';
import { api, ApiError } from '../api/client';
import { useRef } from 'react';
import { useKeepFocus } from '../common/focusLost';
import { useAction } from '../common/useAction';
import type { ConfigCommand } from '../api/gen/ConfigCommand';
import { invalidateCommands, runKeys } from './queries';

type Confirm = (o: ConfirmOptions) => Promise<ConfirmResult>;

/** A command as it runs: its setup, then the command with its environment. */
export function commandText(c: ConfigCommand): string {
  const env = Object.entries(c.env).map(([k, v]) => `${k}=${v} `).join('');
  return c.setup ? `${c.setup}\n${env}${c.command}` : `${env}${c.command}`;
}

/** The commands, each in a card saying what uses it. */
function CommandCards({ commands }: { commands: ConfigCommand[] }) {
  return (
    <div className="flex flex-col gap-2">
      {commands.map((c) => (
        <CodeCard key={c.fingerprint} title={c.uses.join(', ')} code={commandText(c)} aria-label={`Command for ${c.uses.join(', ')}`} />
      ))}
    </div>
  );
}

/** The commands an `untrusted_commands` error carries, or null for any other error. */
function untrustedCommands(e: unknown): { projectId: string; commands: ConfigCommand[] } | null {
  if (!(e instanceof ApiError) || e.code !== 'untrusted_commands') return null;
  const d = e.data as { projectId?: string; commands?: ConfigCommand[] } | undefined;
  return d?.projectId && d.commands?.length ? { projectId: d.projectId, commands: d.commands } : null;
}

/** Runs `attempt`. When it needs commands the user hasn't approved, shows them and, once the user approves them (which
 * refreshes the project's commands), tries again; declining resolves to null without an error. */
export async function withApproval<T>(confirm: Confirm, qc: QueryClient, attempt: () => Promise<T>): Promise<T | null> {
  try {
    return await attempt();
  } catch (e) {
    const waiting = untrustedCommands(e);
    if (!waiting) throw e;
    const one = waiting.commands.length === 1;
    const { ok } = await confirm({
      title: one ? 'Run this command?' : 'Run these commands?',
      body: (
        <div className="flex flex-col gap-3">
          <p>
            {one ? 'It comes' : 'They come'} from the project's run configuration and PDT hasn't run {one ? 'it' : 'them'} before.
            A repository, a branch or an agent can change these commands, so only approve what you trust.
          </p>
          <CommandCards commands={waiting.commands} />
        </div>
      ),
      action: 'Approve and run',
    });
    if (!ok) return null;
    await api.trustCommands(waiting.projectId, waiting.commands.map((c) => c.fingerprint));
    void invalidateCommands(qc, waiting.projectId);
    return attempt();
  }
}

/** A command with whether it waits on the user's approval, and Approve when it does. */
function CommandRow({ command: c, waiting, approve, approving }: {
  command: ConfigCommand;
  waiting: boolean;
  approve: (fingerprints: string[], from: Element) => void;
  approving: boolean;
}) {
  return (
    <CodeCard
      aria-label={`Command for ${c.uses.join(', ')}`}
      tabIndex={-1}
      data-fingerprint={c.fingerprint}
      title={c.uses.join(', ')}
      aside={waiting ? (
        <span className="flex items-center gap-2">
          <Badge tone="waiting">Not approved</Badge>
          <Button type="button" size="sm" data-approve pending={approving} onClick={(e) => approve([c.fingerprint], e.target)}>Approve</Button>
        </span>
      ) : 'Approved'}
      code={commandText(c)}
    />
  );
}

/** The commands the main checkout's run configuration runs (with the overrides), those waiting on the user's approval
 * first, each with Approve; then the commands waiting in the project's worktrees (an agent's new or changed
 * configuration), under their branch and ticket. */
export function ConfigCommands({ projectId }: { projectId: string }) {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: runKeys.commands(projectId), queryFn: () => api.projectCommands(projectId, null) });
  // Approve goes away once it lands, and the command moves down the list: focus goes to the next Approve, or the
  // command's card (PDT-668).
  const keep = useKeepFocus();
  const box = useRef<HTMLDivElement>(null);
  const [approve, approving] = useAction(async (fingerprints: string[], from: Element) => {
    keep(from, () => box.current?.querySelector<HTMLElement>('[data-approve]')
      ?? box.current?.querySelector<HTMLElement>(`[data-fingerprint="${CSS.escape(fingerprints[0])}"]`));
    await api.trustCommands(projectId, fingerprints);
    await invalidateCommands(qc, projectId);
  }, { success: 'Approved' });
  if (list.isPending) return <Skeleton lines={2} label="Loading the commands" />;
  if (list.isError) return <Alert tone="error">{list.error.message}</Alert>;
  const waiting = new Set(list.data.untrusted);
  const ordered = [...list.data.commands].sort((a, b) => Number(waiting.has(b.fingerprint)) - Number(waiting.has(a.fingerprint)));
  const worktrees = list.data.worktrees;
  const main = list.data.error ? null : ordered.length === 0
    ? <p className="text-xs text-muted-foreground">The run configuration names no commands.</p>
    : ordered.map((c) => (
      <CommandRow key={c.fingerprint} command={c} waiting={waiting.has(c.fingerprint)} approve={(f, from) => void approve(f, from)} approving={approving} />
    ));
  if (!main && worktrees.length === 0) return null;
  return (
    <div ref={box} className="flex flex-col gap-2">
      {main}
      {worktrees.map((w) => (
        <section key={w.worktreeId} aria-label={`Waiting in ${w.branch}`} className="flex flex-col gap-2">
          <SectionLabel as="h4" className="pt-2">{w.ticketKey ? `${w.ticketKey} · ` : ''}Waiting in {w.branch}</SectionLabel>
          {w.commands.map((c) => (
            <CommandRow key={c.fingerprint} command={c} waiting approve={(f, from) => void approve(f, from)} approving={approving} />
          ))}
        </section>
      ))}
    </div>
  );
}
