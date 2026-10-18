import { useState, type ReactNode } from 'react';
import { FolderGit2, GitBranch, Inbox, Tag } from 'lucide-react';
import { Button } from '@ui/Button';
import { StatusDot, type Status } from '@ui/StatusDot';
import { Badge } from '@ui/Badge';
import { Chip } from '@ui/Chip';
import { Alert } from '@ui/Alert';
import { Banner } from '@ui/Banner';
import { EmptyState } from '@ui/EmptyState';
import { Toaster, toast } from '@ui/Toaster';
import { Dialog } from '@ui/Dialog';
import { ConfirmProvider, useConfirm } from '@ui/Confirm';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}

const statuses: Status[] = ['running', 'waiting', 'failed', 'idle', 'ended'];

function DialogDemo() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Open dialog</Button>
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title="Add project"
        description="A git repository on this machine, or a URL to clone."
        footer={
          <>
            <Button variant="secondary" onClick={() => setOpen(false)}>Cancel</Button>
            <Button variant="primary" onClick={() => setOpen(false)}>Add project</Button>
          </>
        }
      >
        <p className="text-sm text-muted-foreground">The body: a form, a list, anything. Escape, the backdrop or × closes it.</p>
      </Dialog>
      <Dialog trigger={<Button variant="ghost">Dialog with trigger</Button>} title="Keyboard shortcuts" size="sm">
        <p className="text-sm">Press Ctrl+] then one key.</p>
      </Dialog>
    </>
  );
}

function ConfirmDemo() {
  const confirm = useConfirm();
  const [last, setLast] = useState('');
  return (
    <>
      <Button
        variant="danger"
        onClick={async () => {
          const r = await confirm({
            title: 'Remove worktree feature-x?',
            body: 'It has uncommitted changes; they will be lost.',
            action: 'Remove worktree',
            danger: true,
            checkbox: { label: 'Also delete the branch' },
          });
          setLast(JSON.stringify(r));
        }}
      >
        Remove worktree…
      </Button>
      <Button
        onClick={async () => setLast(JSON.stringify(await confirm({ title: 'Restart the agent?', body: 'It resumes its conversation.', action: 'Restart' })))}
      >
        Restart…
      </Button>
      {last && <code className="font-mono text-meta text-muted-foreground">{last}</code>}
    </>
  );
}

function render() {
  return (
    <ConfirmProvider>
      <Toaster />
      <div className="flex flex-col gap-5">
        <Row label="StatusDot">
          {statuses.map((s) => (
            <span key={s} className="flex items-center gap-1.5 text-xs">
              <StatusDot status={s} /> {s}
            </span>
          ))}
          <span className="flex items-center gap-1.5 text-xs">
            <StatusDot status="running" pulse label="Working" /> running + pulse
          </span>
        </Row>

        <Row label="Badge">
          <Badge>neutral</Badge>
          <Badge tone="running">Working</Badge>
          <Badge tone="waiting">Needs you</Badge>
          <Badge tone="failed">Failed</Badge>
          <Badge tone="accent">New</Badge>
          <Badge count={3} aria-label="3 unread" />
          <Badge tone="waiting" count={12} aria-label="12 waiting" />
          <Badge tone="accent" count={250} aria-label="250 new" />
        </Row>

        <Row label="Chip">
          <Chip>PDT</Chip>
          <Chip icon={<GitBranch aria-hidden />}>feature/long-branch-name</Chip>
          <Chip icon={<GitBranch aria-hidden />} href="https://github.com/">main</Chip>
          <Chip icon={<Tag aria-hidden />}>bug</Chip>
          <Chip href="#PDT-42">PDT-42</Chip>
        </Row>

        <Row label="Alert">
          <div className="flex w-full max-w-lg flex-col gap-2">
            <Alert tone="error" title="Could not add the project" action={<Button size="sm" variant="secondary">Retry</Button>}>
              Not a git repository: /Users/me/code/thing
            </Alert>
            <Alert tone="warning">The worktree has uncommitted changes.</Alert>
            <Alert tone="info" title="Remote access is off">Turn it on in Host settings to reach PDT from your phone.</Alert>
          </div>
        </Row>

        <Row label="Banner (fixed h-8, never wraps)">
          <div className="flex w-full flex-col overflow-hidden rounded-md border border-border">
            <Banner tone="accent" actions={<><Button size="sm" variant="ghost">Release notes</Button><Button size="sm" variant="primary">Update</Button></>}>
              Update to v0.1.99 available
            </Banner>
            <Banner tone="running" actions={<Button size="sm" variant="ghost">Dismiss</Button>}>Updated to v0.1.99.</Banner>
            <Banner tone="waiting">Installing the update; this page reconnects when the host is back.</Banner>
            <Banner tone="failed" title="Update failed: health check timed out after 30 s and the previous version was restored" actions={<Button size="sm" variant="ghost">Dismiss</Button>}>
              Update failed: health check timed out after 30 s and the previous version was restored
            </Banner>
            <Banner>A neutral notice.</Banner>
          </div>
        </Row>

        <Row label="EmptyState">
          <div className="grid w-full gap-3 sm:grid-cols-2">
            <EmptyState icon={<Inbox />} action={<Button variant="primary">Start an agent</Button>} className="border border-border">
              No agents in this workspace.
            </EmptyState>
            <EmptyState icon={<FolderGit2 />} className="border border-border">No projects yet.</EmptyState>
          </div>
        </Row>

        <Row label="Toast">
          <Button onClick={() => toast.success('Saved')}>Success</Button>
          <Button onClick={() => toast.error('Could not stop the run: process not found', { action: { label: 'Retry', onClick: () => toast.info('Retrying…') } })}>
            Error with action
          </Button>
          <Button onClick={() => toast.info('Branch pushed to origin')}>Info</Button>
        </Row>

        <Row label="Dialog">
          <DialogDemo />
        </Row>

        <Row label="useConfirm">
          <ConfirmDemo />
        </Row>
      </div>
    </ConfirmProvider>
  );
}

export const section = { title: 'Status and feedback', order: 20, render };
