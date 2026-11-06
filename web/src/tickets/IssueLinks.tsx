// A ticket's issues in other trackers and its plain links, in its Links section (PDT-481): "Link issue…" takes an
// issue's address or key, which each tracker plugin recognizes for itself; an address none knows can be added as a
// plain labelled link. A linked issue shows its tracker's icon, title and state (read again when the ticket is opened),
// and one linked by hand has "Close it when this ticket is done", off until the user turns it on.
import { useEffect, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { cn } from '@ui/lib/utils';
import { Alert, Badge, Button, Checkbox, Chip, Dialog, Field, Input, MenuButton } from '@ui';
import { api } from '../api/client';
import { ApiError } from '../api/core';
import type { ExternalLink } from '../api/gen/ExternalLink';
import type { Ticket } from '../api/gen/Ticket';
import type { WebLink } from '../api/gen/WebLink';
import { useAction } from '../common/useAction';
import { PluginIcon } from '../plugins/parts';
import type { Plugin } from '../plugins/model';
import { issueTitle, shortKey, usePluginList } from './ExternalKeys';
import { invalidateTickets, ticketKeys } from './queries';

/** Reads the ticket's linked issues again when it is opened (the server reads only those a few minutes old). */
export function useRefreshIssues(ticket: Ticket) {
  const qc = useQueryClient();
  const has = ticket.external.length > 0;
  useEffect(() => {
    if (!has) return;
    let live = true;
    // Only this ticket's issues change: it takes the answer, and no list is read again.
    api.refreshIssues(ticket.id).then(
      (fresh) => { if (live) qc.setQueryData(ticketKeys.one(fresh.id), fresh); },
      () => { /* The chips keep what they had; the next open tries again. */ },
    );
    return () => { live = false; };
  }, [ticket.id, has, qc]);
}

/** The ticket's issues, each with its tracker, title, state and menu. */
export function IssueRows({ ticket }: { ticket: Ticket }) {
  const plugins = usePluginList();
  if (ticket.external.length === 0) return null;
  return (
    <div className="flex flex-col gap-1">
      <h4 className="text-meta text-muted-foreground">Issues</h4>
      <ul aria-label="Issues" className="flex flex-col gap-1.5">
        {ticket.external.map((l) => (
          <IssueRow key={`${l.system}:${l.key}`} ticket={ticket} link={l} plugin={plugins?.find((p) => p.id === l.system)} />
        ))}
      </ul>
    </div>
  );
}

function IssueRow({ ticket, link, plugin }: { ticket: Ticket; link: ExternalLink; plugin: Plugin | undefined }) {
  const qc = useQueryClient();
  const [unlink] = useAction(async () => {
    await api.unlinkIssue(ticket.id, { system: link.system, key: link.key });
    invalidateTickets(qc, [ticket.id]);
  });
  const [setClose, setting] = useAction(async (on: boolean) => {
    await api.setIssueClose(ticket.id, { system: link.system, key: link.key, closeOnDone: on });
    invalidateTickets(qc, [ticket.id]);
  });
  const closed = link.state === 'closed';
  // Issues PDT filed or imported always close; one linked by hand asks, where its tracker can close it.
  const offersClose = link.origin === 'manual' && !!plugin?.issues?.closesOnDone;
  return (
    <li className="flex min-w-0 flex-col gap-1 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        {plugin && <PluginIcon p={plugin} size="xs" />}
        <Chip href={link.url} title={issueTitle(link, plugin)}>{shortKey(link)}</Chip>
        <span className={cn('min-w-0 flex-1 truncate', closed && 'text-muted-foreground line-through')} title={link.title ?? undefined}>
          {link.title ?? link.key}
        </span>
        {link.state && <Badge tone={closed ? 'neutral' : 'running'}>{closed ? 'Closed' : 'Open'}</Badge>}
        <MenuButton
          label={`Issue ${link.key}`}
          size="sm"
          entries={() => [{ label: 'Unlink', danger: true, onSelect: () => void unlink() }]}
        />
      </div>
      {offersClose && (
        <Checkbox checked={link.closeOnDone} disabled={setting} onChange={(on) => void setClose(on)} className="pl-6">
          Close it when this ticket is done
        </Checkbox>
      )}
    </li>
  );
}

/** The ticket's plain links (docs, designs). */
export function WebLinkRows({ ticket }: { ticket: Ticket }) {
  if (ticket.webLinks.length === 0) return null;
  return (
    <div className="flex flex-col gap-1">
      <h4 className="text-meta text-muted-foreground">Other links</h4>
      <ul aria-label="Other links" className="flex flex-col gap-1">
        {ticket.webLinks.map((l) => <WebLinkRow key={l.id} ticket={ticket} link={l} />)}
      </ul>
    </div>
  );
}

function WebLinkRow({ ticket, link }: { ticket: Ticket; link: WebLink }) {
  const qc = useQueryClient();
  const [remove] = useAction(async () => {
    await api.removeWebLink(ticket.id, link.id);
    invalidateTickets(qc, [ticket.id]);
  });
  return (
    <li className="flex min-w-0 items-center gap-2 text-xs">
      <a href={link.url} target="_blank" rel="noreferrer" title={link.url} className="min-w-0 flex-1 truncate text-link hover:underline">
        {link.label}
      </a>
      <MenuButton label={`Link ${link.label}`} size="sm" entries={() => [{ label: 'Remove link', danger: true, onSelect: () => void remove() }]} />
    </li>
  );
}

const isAddress = (s: string) => /^https?:\/\/\S+$/i.test(s.trim());

/** Paste an issue's address or key; an address no tracker knows can be added as a plain link with a label. */
export function LinkIssueDialog({ ticket, onClose }: { ticket: Ticket; onClose: () => void }) {
  const qc = useQueryClient();
  const plugins = usePluginList();
  const [given, setGiven] = useState('');
  const [label, setLabel] = useState('');
  const examples = (plugins ?? []).filter((p) => p.enabled && p.issues).map((p) => `${p.label}: ${p.issues!.example}`);
  const done = () => { invalidateTickets(qc, [ticket.id]); onClose(); };
  const linkIt = useMutation({ meta: { errorShown: true }, mutationFn: () => api.linkIssue(ticket.id, { issue: given.trim() }), onSuccess: done });
  const addPlain = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.addWebLink(ticket.id, { url: given.trim(), label: label.trim() || undefined }),
    onSuccess: done,
  });
  const unrecognized = linkIt.error instanceof ApiError && linkIt.error.code === 'unrecognized';
  const plain = unrecognized && isAddress(given);
  const ready = given.trim() !== '';
  const submit = () => {
    if (!ready) return;
    if (plain) { if (!addPlain.isPending) addPlain.mutate(); } else if (!linkIt.isPending) linkIt.mutate();
  };
  const reset = () => { linkIt.reset(); addPlain.reset(); };
  return (
    <Dialog
      onClose={onClose}
      title={`Link an issue to ${ticket.key}`}
      description="Paste the issue's address or its key. Done leaves it as it is unless you turn on closing it."
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          {plain
            ? <Button variant="primary" pending={addPlain.isPending} onClick={submit}>Add link</Button>
            : <Button variant="primary" disabled={!ready} pending={linkIt.isPending} onClick={submit}>Link</Button>}
        </>
      }
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <Field label="Issue" hint={examples.length ? examples.join(' · ') : undefined}>
          <Input
            aria-label="Issue"
            placeholder="https://… or ABC-123"
            autoFocus
            value={given}
            onChange={(e) => { setGiven(e.target.value); reset(); }}
          />
        </Field>
        {plain && (
          <>
            <Alert tone="info">No tracker recognizes this address. Add it as a plain link instead.</Alert>
            <Field label="Label" hint="Leave it empty to show the site's name.">
              <Input aria-label="Label" placeholder="Docs, Figma…" value={label} onChange={(e) => setLabel(e.target.value)} />
            </Field>
          </>
        )}
        {linkIt.error && !plain && <Alert tone="error">{linkIt.error.message}</Alert>}
        {addPlain.error && <Alert tone="error">{addPlain.error.message}</Alert>}
      </form>
    </Dialog>
  );
}
