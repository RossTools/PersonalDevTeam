// The Enabled switch (Phase 7d spec §4): turning a workflow on is consent to what it does, so it says what that is
// (its triggers and actions) and offers a dry run first. Turning it off is immediate.
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Switch } from '@ui';
import { api } from '../api/client';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';
import { summary, type Catalog } from './model';

export function EnableSwitch({ w, catalog, blocked, onDryRun }: {
  w: WorkflowDetail;
  catalog: Catalog;
  /** Why it can't be enabled now (unsaved changes, errors). */
  blocked?: string | null;
  onDryRun: () => void;
}) {
  const qc = useQueryClient();
  const [asking, setAsking] = useState(false);
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => api.patchWorkflow(w.id, { enabled }),
    onSuccess: () => {
      setAsking(false);
      void qc.invalidateQueries({ queryKey: ['workflows'] });
      void qc.invalidateQueries({ queryKey: ['workflow', w.id] });
    },
  });
  const { triggers, actions } = summary(w.doc, catalog);
  return (
    <>
      <Switch aria-label="Enabled" checked={w.enabled} disabled={!w.enabled && !!blocked}
        onChange={(on) => (on ? setAsking(true) : toggle.mutate(false))}>
        <span className="text-sm">{w.enabled ? 'On' : 'Off'}</span>
      </Switch>
      <Dialog
        open={asking}
        onOpenChange={setAsking}
        title={`Turn on ${w.name}?`}
        description="While it is on, PDT runs it by itself whenever its trigger happens."
        footer={
          <>
            <Button variant="secondary" onClick={() => setAsking(false)}>Cancel</Button>
            <Button variant="secondary" onClick={() => { setAsking(false); onDryRun(); }}>Dry run first</Button>
            <Button variant="primary" onClick={() => toggle.mutate(true)} pending={toggle.isPending}>Turn on</Button>
          </>
        }
      >
        <div className="flex flex-col gap-2 text-sm">
          <p><span className="font-medium">Runs on:</span> {triggers.length ? triggers.join(', ') : 'nothing (Run now only)'}</p>
          <p><span className="font-medium">Can:</span> {actions.length ? actions.join(', ') : 'nothing that changes anything'}</p>
          {toggle.error && <Alert tone="error">{toggle.error.message}</Alert>}
        </div>
      </Dialog>
      {!asking && toggle.error && <span className="text-meta text-destructive">{toggle.error.message}</span>}
    </>
  );
}
