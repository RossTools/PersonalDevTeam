// Import from Jira… (Phase 7e): one Jira issue by its key, assigned to you or not, as a Draft ticket linked to it. An
// issue a ticket already links is not imported again: that ticket opens instead.
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input } from '@ui';
import { api } from '../api/client';
import { useOpenTicket } from './actions';
import { invalidateTickets } from './queries';

export function ImportJiraDialog({ projectId, onClose }: { projectId: string; onClose: () => void }) {
  const qc = useQueryClient();
  const open = useOpenTicket();
  const [key, setKey] = useState('');
  const run = useMutation({
    mutationFn: () => api.importJiraIssue(projectId, key.trim()),
    onSuccess: (r) => {
      invalidateTickets(qc, [r.ticket.id]);
      open(r.ticket);
      onClose();
    },
  });
  const ready = key.trim() !== '';
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      title="Import from Jira"
      description="Makes a Draft ticket from a Jira issue, linked to it. An issue that already has a ticket opens that ticket."
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!ready} pending={run.isPending} onClick={() => run.mutate()}>Import</Button></>}
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); if (ready && !run.isPending) run.mutate(); }}>
        <Field label="Issue key">
          <Input aria-label="Issue key" placeholder="ABC-123" autoFocus value={key} onChange={(e) => { setKey(e.target.value); run.reset(); }} />
        </Field>
        {run.error && <Alert tone="error">{run.error.message}</Alert>}
      </form>
    </Dialog>
  );
}
