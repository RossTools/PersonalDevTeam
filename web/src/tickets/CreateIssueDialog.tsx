// Create GitHub issue (PDT-221): files an issue in the project's repository as the user's GitHub login, from a ticket
// (pre-filled with its title and description; the ticket then links the issue) or new (the palette's "New issue").
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input, Textarea, toast } from '@ui';
import { api } from '../api/client';
import type { Ticket } from '../api/gen/Ticket';
import { invalidateTickets } from './queries';
import { openExternal } from '../common/open';

/** GitHub's longest issue title (crates/pdt-api/src/issues.rs). */
const MAX_TITLE = 256;

export function CreateIssueDialog({ projectId, repo, ticket, onClose }: {
  projectId: string;
  /** `owner/name`, as the project's forge names it. */
  repo: string;
  /** The ticket it is filed from: it pre-fills the form and links the new issue. */
  ticket?: Pick<Ticket, 'id' | 'key' | 'title' | 'description'>;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [title, setTitle] = useState(ticket?.title ?? '');
  const [body, setBody] = useState(ticket?.description ?? '');
  const run = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.createIssue(projectId, { title: title.trim(), body, labels: [], ...(ticket ? { ticketId: ticket.id } : {}) }),
    onSuccess: (r) => {
      if (ticket) invalidateTickets(qc, [ticket.id]);
      toast.success(`Filed ${r.repo}#${r.number}`, {
        ...(r.ticketKey ? { description: `${r.ticketKey} links it.` } : {}),
        action: { label: 'Open', onClick: () => openExternal(r.url) },
      });
      onClose();
    },
  });
  const ready = title.trim() !== '' && title.trim().length <= MAX_TITLE;
  const send = () => { if (ready && !run.isPending) run.mutate(); };
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      title={ticket ? `Create GitHub issue from ${ticket.key}` : 'New GitHub issue'}
      description={`Opens an issue in ${repo} as your GitHub login.${ticket ? ` ${ticket.key} links it.` : ''}`}
      size="lg"
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!ready} pending={run.isPending} onClick={send}>Create issue</Button></>}
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); send(); }}>
        <Field label="Title">
          <Input aria-label="Title" autoFocus maxLength={MAX_TITLE} value={title} onChange={(e) => { setTitle(e.target.value); run.reset(); }} />
        </Field>
        <Field label="Description">
          <Textarea
            aria-label="Description"
            rows={8}
            value={body}
            placeholder="Markdown"
            onChange={(e) => { setBody(e.target.value); run.reset(); }}
            className="min-h-32 resize-y"
          />
        </Field>
        {run.error && <Alert tone="error">{run.error.message}</Alert>}
      </form>
    </Dialog>
  );
}
