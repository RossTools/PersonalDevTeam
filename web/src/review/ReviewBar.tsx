import type { ReactNode } from 'react';
import { Check, GitMerge, Send } from 'lucide-react';
import { Badge, Button } from '@ui';

export interface ReviewBarProps {
  drafts: number;
  onSend: () => void;
  sending: boolean;
  /** Approve and Merge… are offered for an active attempt. */
  attemptActive: boolean;
  approved: boolean;
  onApprove: () => void;
  onMerge: () => void;
  /** Before the actions (the files button on phones, the PR badge). */
  leading?: ReactNode;
}

/** The review's actions: "N draft comments" → Send feedback, Approve, Merge…. */
export function ReviewBar({ drafts, onSend, sending, attemptActive, approved, onApprove, onMerge, leading }: ReviewBarProps) {
  return (
    <div role="toolbar" aria-label="Review" className="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1">
      {leading}
      <span className="ml-auto hidden text-xs text-muted-foreground sm:inline" aria-live="polite">
        {drafts === 0 ? 'No draft comments' : `${drafts} draft comment${drafts === 1 ? '' : 's'}`}
      </span>
      <Button variant="primary" size="sm" disabled={drafts === 0} pending={sending} onClick={onSend} className="ml-auto sm:ml-0">
        <Send aria-hidden /> Send feedback
        {drafts > 0 && <span className="sm:hidden">({drafts})</span>}
      </Button>
      {attemptActive && (
        approved ? (
          <Badge tone="running"><Check aria-hidden /> Approved</Badge>
        ) : (
          <Button size="sm" onClick={onApprove}><Check aria-hidden /> Approve</Button>
        )
      )}
      {attemptActive && (
        <Button size="sm" onClick={onMerge}><GitMerge aria-hidden /> Merge…</Button>
      )}
    </div>
  );
}
