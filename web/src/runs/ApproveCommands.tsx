import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, CodeCard, Skeleton, type ConfirmOptions, type ConfirmResult } from '@ui';
import { api, ApiError } from '../api/client';
import { useAction } from '../common/useAction';
import type { ConfigCommand } from '../api/gen/ConfigCommand';
import { runKeys } from './queries';

type Confirm = (o: ConfirmOptions) => Promise<ConfirmResult>;

/** A command as it runs: its setup, then the command with its environment. */
function commandText(c: ConfigCommand): string {
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

/** Runs `attempt`. When it needs commands the user hasn't approved, shows them and, once the user approves them, tries
 * again; declining resolves to null without an error. */
export async function withApproval<T>(confirm: Confirm, attempt: () => Promise<T>): Promise<T | null> {
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
    return attempt();
  }
}

/** The commands the main checkout's run configuration runs (with the overrides), those waiting on the user's approval
 * first, each with Approve. */
export function ConfigCommands({ projectId }: { projectId: string }) {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: runKeys.commands(projectId), queryFn: () => api.projectCommands(projectId, null) });
  const [approve, approving] = useAction(async (fingerprints: string[]) => {
    await api.trustCommands(projectId, fingerprints);
    await qc.invalidateQueries({ queryKey: runKeys.commands(projectId) });
  }, { success: 'Approved' });
  if (list.isPending) return <Skeleton lines={2} label="Loading the commands" />;
  if (list.isError) return <Alert tone="error">{list.error.message}</Alert>;
  if (list.data.error) return null;
  const waiting = new Set(list.data.untrusted);
  if (list.data.commands.length === 0) return <p className="text-xs text-muted-foreground">The run configuration names no commands.</p>;
  const ordered = [...list.data.commands].sort((a, b) => Number(waiting.has(b.fingerprint)) - Number(waiting.has(a.fingerprint)));
  return (
    <div className="flex flex-col gap-2">
      {ordered.map((c) => (
        <CodeCard
          key={c.fingerprint}
          aria-label={`Command for ${c.uses.join(', ')}`}
          title={c.uses.join(', ')}
          aside={waiting.has(c.fingerprint) ? (
            <span className="flex items-center gap-2">
              <Badge tone="waiting">Not approved</Badge>
              <Button type="button" size="sm" pending={approving} onClick={() => void approve([c.fingerprint])}>Approve</Button>
            </span>
          ) : 'Approved'}
          code={commandText(c)}
        />
      ))}
    </div>
  );
}
