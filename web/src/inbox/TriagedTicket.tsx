import type { InboxItem } from '../api/gen/InboxItem';
import { Markdown } from '../common/Markdown';
import { triageOf } from './model';

/**
 * A ticket triage drafted, waiting for the user's decision (PDT-225): what Approve and Reject do, triage's notes and
 * the issue it came from.
 */
export function TriagedTicket({ item }: { item: InboxItem }) {
  const { summary, issue, external } = triageOf(item);
  return (
    <>
      <p className="text-sm">
        {typeof item.body.workflow === 'string' ? item.body.workflow : 'Triage'} drafted this ticket. Approve moves it to
        To do{external ? ' and lets it in: it came from outside PDT, so only you can' : ''}; Reject moves it to Won&apos;t
        do. Approve doesn&apos;t label it agentic. Moving it on the board does the same and takes it out of the Inbox.
      </p>
      {issue && <div className="text-sm text-muted-foreground"><Markdown>{`From [${issue}](${issue})`}</Markdown></div>}
      {summary && <div className="rounded-md border border-border p-3 text-sm"><Markdown>{summary}</Markdown></div>}
    </>
  );
}
