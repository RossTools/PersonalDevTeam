import { useContext, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Field, Input, Link, Skeleton, Textarea } from '@ui';
import { ApiError, api } from '../api/client';
import type { BugReportFiled } from '../api/gen/BugReportFiled';
import type { BugReportPreview } from '../api/gen/BugReportPreview';
import { actionError } from '../common/useAction';
import { OpenTicketContext } from '../tickets/actions';

/** Error codes whose fix is in Host settings (the token, or the bug reports setting). */
const SETTINGS_CODES = ['no_token', 'token_rejected', 'token_no_issue_access', 'bug_reports_off', 'no_bug_project'];

/** Where a report will go, said before it is sent. */
function destination(p: BugReportPreview) {
  switch (p.route) {
    case 'github':
      return <>It becomes a public issue on <span className="font-medium">{p.repo}</span> on GitHub, filed with this host's update token.</>;
    case 'local':
      return <>It becomes a Draft ticket labelled <span className="font-medium">bug</span> in {p.project ? <span className="font-medium">{p.project.key} · {p.project.name}</span> : 'the PDT project'} on this instance. Nothing is sent to GitHub.</>;
    case 'off':
      return <>Bug reports are turned off in Host settings.</>;
  }
}

export interface BugReportDialogProps {
  onClose: () => void;
  /** Opens Host settings (where the token and the bug reports setting are). */
  onOpenSettings: () => void;
}

/** "Report a bug" (PDT-153): title, description, optional diagnostics with their preview; then a link to what it filed. */
export function BugReportDialog({ onClose, onOpenSettings }: BugReportDialogProps) {
  const openTicket = useContext(OpenTicketContext);
  const preview = useQuery({ queryKey: ['bug-report-preview'], queryFn: api.bugReportPreview });
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [diagnostics, setDiagnostics] = useState(true);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [error, setError] = useState<{ message: string; settings: boolean } | null>(null);
  const [pending, setPending] = useState(false);
  const [filed, setFiled] = useState<BugReportFiled | null>(null);
  // One per form: a resend after a dropped answer returns the first report instead of filing another.
  const nonce = useRef(crypto.randomUUID());
  const p = preview.data;
  const ready = !!title.trim() && !!p && p.route !== 'off';

  const send = async () => {
    if (!ready || pending) return;
    setPending(true);
    setError(null);
    try {
      setFiled(await api.reportBug({ title: title.trim(), description, includeDiagnostics: diagnostics, nonce: nonce.current }));
    } catch (e) {
      setError({ message: actionError(e), settings: e instanceof ApiError && SETTINGS_CODES.includes(e.code) });
      requestAnimationFrame(() => document.getElementById('bug-report-error')?.focus());
    } finally {
      setPending(false);
    }
  };

  if (filed) {
    return (
      <Dialog open onOpenChange={(o) => !o && onClose()} title="Bug reported" size="md" footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
        {filed.kind === 'issue' ? (
          <p className="text-sm">
            Thanks. It is issue <Link external href={filed.url}>#{filed.number}</Link> on GitHub.
          </p>
        ) : (
          <p className="text-sm">
            Thanks. It is ticket{' '}
            <Link onClick={() => { openTicket({ id: filed.id, projectId: filed.projectId }); onClose(); }}>{filed.key}</Link>, a Draft on this instance.
          </p>
        )}
      </Dialog>
    );
  }

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      dismissable={!title.trim() && !description.trim()}
      title="Report a bug"
      description="A bug in PDT itself. Say what happened, what you expected, and how to make it happen again."
      size="lg"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          <Button type="submit" form="bug-report" variant="primary" pending={pending} disabled={!ready}>
            {p?.route === 'local' ? 'File ticket' : 'Send report'}
          </Button>
        </>
      }
    >
      <form id="bug-report" className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); void send(); }}>
        {preview.isPending && <Skeleton lines={1} label="Loading where reports go" />}
        {preview.error && <Alert tone="error">Could not load where reports go: {preview.error.message}</Alert>}
        {p && (
          <Alert
            tone={p.route === 'off' ? 'warning' : 'info'}
            action={p.route === 'off' || (p.route === 'github' && !p.tokenPresent) ? <Button size="sm" onClick={onOpenSettings}>Host settings</Button> : undefined}
          >
            {destination(p)}
            {p.route === 'github' && !p.tokenPresent && ' This host has no update token yet: add one in Host settings first.'}
          </Alert>
        )}
        <Field label="Title">
          <Input autoFocus value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)} placeholder="What went wrong, in one line" />
        </Field>
        <Field label="Description">
          <Textarea
            rows={6}
            value={description}
            placeholder="Markdown: what happened, what you expected, steps to repeat it"
            onChange={(e) => setDescription(e.target.value)}
            className="min-h-28 resize-y"
          />
        </Field>
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2">
            <Checkbox className="flex-1" checked={diagnostics} onChange={setDiagnostics} label="Include diagnostics" />
            {p && diagnostics && (
              <Button size="sm" variant="ghost" aria-expanded={showDiagnostics} onClick={() => setShowDiagnostics((s) => !s)}>
                {showDiagnostics ? 'Hide' : 'Show what is sent'}
              </Button>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            PDT's version, OS, update state and the end of its log, with tokens, home folders and this machine's names taken out. No tickets or project content.
          </p>
          {p && diagnostics && showDiagnostics && (
            <pre aria-label="Diagnostics" className="max-h-64 overflow-auto rounded-md border border-border bg-muted p-2 text-xs whitespace-pre-wrap">{p.diagnostics}</pre>
          )}
        </div>
        {error && (
          <Alert
            id="bug-report-error"
            tone="error"
            tabIndex={-1}
            action={error.settings ? <Button size="sm" onClick={onOpenSettings}>Host settings</Button> : undefined}
          >
            {error.message}
          </Alert>
        )}
      </form>
    </Dialog>
  );
}
