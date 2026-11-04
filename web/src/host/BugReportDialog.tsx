import { useContext, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, DialogActions, Field, Input, Link, Skeleton, Textarea } from '@ui';
import { ApiError, api } from '../api/client';
import type { BugReportFiled } from '../api/gen/BugReportFiled';
import type { BugReportPreview } from '../api/gen/BugReportPreview';
import { copyToClipboard } from '../common/clipboard';
import { actionError } from '../common/useAction';
import { OpenTicketContext, openExternal, openPlugin } from '../common/open';
import { bugFormUrl } from './bugForm';
import { hostKeys } from './queries';

/** Error codes whose fix is in Host settings (the token, or the bug reports setting). */
const SETTINGS_CODES = ['token_rejected', 'token_no_issue_access', 'bug_reports_off', 'no_bug_project'];

/** Where a report will go, said before it is sent. */
function destination(p: BugReportPreview) {
  switch (p.route) {
    case 'github': {
      const repo = <span className="font-medium">{p.repo}</span>;
      switch (p.via) {
        case 'login':
          return <>It becomes a public issue on {repo} on GitHub, filed as {p.githubLogin ? <span className="font-medium">{p.githubLogin}</span> : 'your GitHub login'}.</>;
        case 'token':
          return <>It becomes a public issue on {repo} on GitHub, filed with this host's update token.</>;
        case 'browser':
          return <>PDT isn't connected to GitHub, so it opens the bug form on {repo} filled in, for you to send from your browser. Connect GitHub to file reports from here.</>;
      }
    }
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

/** "Report a bug" (PDT-153): title, the bug form's sections (what happened, expected, how to reproduce; the same as
 *  the GitHub issue form), optional diagnostics with their preview; then a link to what it filed. */
export function BugReportDialog({ onClose, onOpenSettings }: BugReportDialogProps) {
  const openTicket = useContext(OpenTicketContext);
  const preview = useQuery({ queryKey: hostKeys.bugReportPreview, queryFn: api.bugReportPreview });
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [expected, setExpected] = useState('');
  const [steps, setSteps] = useState('');
  const [diagnostics, setDiagnostics] = useState(true);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [error, setError] = useState<{ message: string; settings: boolean } | null>(null);
  const [pending, setPending] = useState(false);
  const [filed, setFiled] = useState<BugReportFiled | null>(null);
  /** The bug form opened on GitHub (no login or token to file with), and whether the diagnostics were copied. */
  const [opened, setOpened] = useState<{ url: string; copied: boolean } | null>(null);
  // One per form: a resend after a dropped answer returns the first report instead of filing another.
  const nonce = useRef(crypto.randomUUID());
  const p = preview.data;
  const ready = !!title.trim() && !!p && p.route !== 'off';
  const inBrowser = p?.route === 'github' && p.via === 'browser';

  /** Opens GitHub's bug form with the report in it, from the click (a pop-up after an await is blocked). The
   *  diagnostics are too long for the link: they go on the clipboard. */
  const openForm = (p: BugReportPreview) => {
    const url = bugFormUrl({
      repo: p.repo, title, whatHappened: description, expected, steps, version: p.version, os: diagnostics ? p.os : undefined,
    });
    openExternal(url);
    if (diagnostics) void copyToClipboard(p.diagnostics, 'Diagnostics copied');
    setOpened({ url, copied: diagnostics });
  };

  const send = async () => {
    if (!ready || pending) return;
    if (inBrowser) return openForm(p);
    setPending(true);
    setError(null);
    try {
      setFiled(await api.reportBug({ title: title.trim(), description, expected, steps, includeDiagnostics: diagnostics, nonce: nonce.current }));
    } catch (e) {
      setError({ message: actionError(e), settings: e instanceof ApiError && SETTINGS_CODES.includes(e.code) });
      requestAnimationFrame(() => document.getElementById('bug-report-error')?.focus());
    } finally {
      setPending(false);
    }
  };

  if (opened) {
    return (
      <Dialog onClose={onClose} title="Finish on GitHub" size="md" footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
        <p className="text-sm">
          GitHub's bug form opened in a new tab with your report in it (<Link external href={opened.url}>open it again</Link>).
          {opened.copied && ' The diagnostics are on the clipboard: paste them at the end of What happened.'} Then submit it there.
        </p>
      </Dialog>
    );
  }

  if (filed) {
    return (
      <Dialog onClose={onClose} title="Bug reported" size="md" footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
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
      onClose={onClose}
      dismissable={![title, description, expected, steps].some((t) => t.trim())}
      title="Report a bug"
      description="A bug in PDT itself. The version, and the OS with the diagnostics, are added for you."
      size="lg"
      footer={
        <DialogActions
          onCancel={onClose}
          action={p?.route === 'local' ? 'File ticket' : inBrowser ? 'Open on GitHub' : 'Send report'}
          form="bug-report"
          pending={pending}
          disabled={!ready}
        />
      }
    >
      <form id="bug-report" className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); void send(); }}>
        {preview.isPending && <Skeleton lines={1} label="Loading where reports go" />}
        {preview.error && <Alert tone="error">Could not load where reports go: {preview.error.message}</Alert>}
        {p && (
          <Alert
            tone={p.route === 'off' ? 'warning' : 'info'}
            action={p.route === 'off' ? <Button size="sm" onClick={onOpenSettings}>Host settings</Button>
              : inBrowser ? <Button size="sm" onClick={() => { openPlugin('github'); onClose(); }}>Connect GitHub</Button>
                : undefined}
          >
            {destination(p)}
          </Alert>
        )}
        <Field label="Title">
          <Input autoFocus value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)} placeholder="What went wrong, in one line" />
        </Field>
        <Field label="What happened">
          <Textarea
            rows={4}
            value={description}
            placeholder="Markdown: what you saw, error messages as they appear"
            onChange={(e) => setDescription(e.target.value)}
            className="min-h-20 resize-y"
          />
        </Field>
        <Field label="What you expected">
          <Textarea rows={2} value={expected} onChange={(e) => setExpected(e.target.value)} className="min-h-12 resize-y" />
        </Field>
        <Field label="How to reproduce">
          <Textarea
            rows={3}
            value={steps}
            placeholder={'1. Open a project.\n2. …'}
            onChange={(e) => setSteps(e.target.value)}
            className="min-h-16 resize-y"
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
            PDT's version, OS, update state and the end of its log, with tokens, home folders, this machine's names and the names of your projects, repositories, branches and worktrees taken out. Read it before you send: the log can still mention other things you work on.
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
