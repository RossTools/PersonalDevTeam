import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query';
import { Button, Disclosure, RelativeTime, SectionLabel, useConfirm } from '@ui';
import { api } from '../api/client';
import type { DocumentChange } from '../api/gen/DocumentChange';
import { Markdown } from '../common/Markdown';
import { TicketKey } from '../common/TicketKey';
import { POLL } from '../common/poll';
import { useAction } from '../common/useAction';
import { documentKeys } from './queries';

/** How many changes a page shows. */
const PAGE = 20;

/** A diff as a Markdown code block, fenced longer than any run of backticks in it. */
function diffBlock(diff: string): string {
  const longest = Math.max(2, ...(diff.match(/`+/g) ?? []).map((r) => r.length));
  const fence = '`'.repeat(longest + 1);
  return `${fence}diff\n${diff.replace(/\n+$/, '')}\n${fence}`;
}

/** Puts the document back as it was before a change, once the user confirms; open views load it again. */
function useRestore(c: DocumentChange) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  return useAction(async () => {
    const { ok } = await confirm({
      title: 'Restore the document as it was before this change?',
      body: 'Every change made since, yours too, is undone with it.',
      action: 'Restore',
      danger: true,
    });
    if (!ok) return;
    await api.restoreDocumentChange(c.documentId, c.id);
    void qc.invalidateQueries({ queryKey: documentKeys.content(c.documentId) });
    void qc.invalidateQueries({ queryKey: documentKeys.version(c.documentId) });
    void qc.invalidateQueries({ queryKey: documentKeys.changes(c.documentId) });
  });
}

function ChangeRow({ c }: { c: DocumentChange }) {
  const [restore, restoring] = useRestore(c);
  return (
    <li className="flex flex-col gap-1 border-t border-border pt-2 first:border-t-0 first:pt-0">
      <p className="flex flex-wrap items-center gap-1.5 text-sm">
        <span className="font-medium">{c.agent.persona ?? 'An agent'}</span>
        {c.agent.fromTicket && <span className="text-muted-foreground">on <TicketKey ticketKey={c.agent.fromTicket} /></span>}
        <span aria-hidden className="text-muted-foreground">·</span>
        <span>{c.summary}</span>
        <span aria-hidden className="text-muted-foreground">·</span>
        <RelativeTime time={c.at} className="text-meta text-muted-foreground" />
        {c.version && <span className="text-meta text-muted-foreground">version {c.version}</span>}
        {c.restorable && (
          <Button size="sm" variant="ghost" className="ml-auto" disabled={restoring} onClick={() => void restore()}>
            Restore
          </Button>
        )}
      </p>
      {c.diff && (
        <Disclosure title="Show the diff" panelClassName="text-sm text-foreground">
          <Markdown>{diffBlock(c.diff)}</Markdown>
        </Disclosure>
      )}
    </li>
  );
}

/**
 * The changes agents made to a document without asking, as its project let them (newest first), each with the agent,
 * its ticket and the diff or version to go back to; a local document's newest changes restore what it was before them.
 * Nothing when there are none.
 */
export function AgentChanges({ id }: { id: string }) {
  const changes = useInfiniteQuery({
    queryKey: documentKeys.changes(id),
    queryFn: ({ pageParam }) => api.documentChanges(id, { limit: PAGE, before: pageParam || undefined }),
    initialPageParam: '' as string,
    getNextPageParam: (last) => (last.length < PAGE ? undefined : last[last.length - 1]?.id),
    refetchInterval: POLL.paged,
  });
  const list = changes.data?.pages.flat() ?? [];
  if (list.length === 0) return null;
  return (
    <section aria-label="Changes agents made without asking" className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
      <SectionLabel as="h2">Changes agents made without asking</SectionLabel>
      <ul className="flex flex-col gap-2">
        {list.map((c) => <ChangeRow key={c.id} c={c} />)}
      </ul>
      {changes.hasNextPage && (
        <Button size="sm" variant="ghost" className="self-start" disabled={changes.isFetchingNextPage} onClick={() => void changes.fetchNextPage()}>
          Show older changes
        </Button>
      )}
    </section>
  );
}
