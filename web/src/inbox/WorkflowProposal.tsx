import type { InboxItem } from '../api/gen/InboxItem';
import { proposalOf } from './model';

/**
 * A workflow version an agent proposed (PDT-320): what Accept and Discard do, the agent's note and what changed. Open
 * shows the whole change in the workflow's Versions tab.
 */
export function WorkflowProposal({ item }: { item: InboxItem }) {
  const { workflow, version, note, summary } = proposalOf(item);
  return (
    <>
      <p className="text-sm">
        Accept makes version {version} the current version of {workflow || 'the workflow'}
        {item.body.enabled === true ? ', which runs from then on' : ''}; Discard sets it aside. Open shows the whole change.
      </p>
      {note && <p aria-label="Note" className="rounded-md border border-border p-3 text-sm whitespace-pre-wrap">{note}</p>}
      {summary && <p aria-label="What changed" className="text-sm text-muted-foreground">{summary}</p>}
    </>
  );
}
