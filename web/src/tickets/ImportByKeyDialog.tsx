// Import one issue by its key (Phase 7e), for each plugin a project is linked to that can: assigned to you or not, as a
// Draft ticket linked to it. An issue a ticket already links is not imported again: that ticket opens instead.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input } from '@ui';
import { api } from '../api/client';
import type { ImportByKey } from '../api/gen/ImportByKey';
import type { ProjectSettingsView } from '../api/gen/ProjectSettingsView';
import { useOpenTicket } from './actions';
import { invalidateTickets } from './queries';

/** A plugin a project can import an issue from, by key. */
export interface IssueImport extends ImportByKey {
  integration: string;
}

/** The imports a project's settings allow: each enabled plugin that declares one and the project is linked to. */
export function useIssueImports(settings: ProjectSettingsView | undefined): IssueImport[] {
  const schemas = useQuery({ queryKey: ['project-settings-schemas'], queryFn: api.projectSettingsSchemas, staleTime: Infinity, enabled: !!settings }).data ?? [];
  return schemas.flatMap((s) => (s.import && settings?.integrations?.[s.integration] ? [{ ...s.import, integration: s.integration }] : []));
}

export function ImportByKeyDialog({ projectId, source, onClose }: { projectId: string; source: IssueImport; onClose: () => void }) {
  const qc = useQueryClient();
  const open = useOpenTicket();
  const [key, setKey] = useState('');
  const run = useMutation({
    mutationFn: () => api.importIssue(projectId, source.integration, key.trim()),
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
      title={source.title}
      description={source.description}
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!ready} pending={run.isPending} onClick={() => run.mutate()}>Import</Button></>}
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); if (ready && !run.isPending) run.mutate(); }}>
        <Field label="Issue key">
          <Input aria-label="Issue key" placeholder={source.placeholder} autoFocus value={key} onChange={(e) => { setKey(e.target.value); run.reset(); }} />
        </Field>
        {run.error && <Alert tone="error">{run.error.message}</Alert>}
      </form>
    </Dialog>
  );
}
