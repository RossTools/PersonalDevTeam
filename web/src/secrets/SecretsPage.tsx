// Settings → Secrets (PDT-158): secrets agents and workflows use by name, never pasted into the chat. A value goes to this machine's
// keychain and is never shown again; the list shows each secret's name, who may use it and its last use, and each
// secret's uses on request. Its description says where values are kept, warning when it's (Linux without a Secret
// Service, PDT-197) a file in the data folder or memory only (PDT-694). On a project's owner it lists, by name only, the secrets its remote
// instances keep for its projects (PDT-637); on a remote instance, a remote project's own show under its name.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, CheckboxPicks, Dialog, Disclosure, DialogActions, EmptyState, Field, Input, Page, RelativeTime, Section, Skeleton, Textarea } from '@ui';
import { api } from '../api/client';
import type { Secret } from '../api/gen/Secret';
import type { SecretStoreInfo } from '../api/gen/SecretStoreInfo';
import { QueryStatus } from '../common/QueryStatus';
import { useConfirmDelete } from '../common/useConfirmDelete';
import { useScopeOptions } from '../common/useScopeOptions';
import { useRemoteProjects } from '../projects/queries';
import { secretKeys, useSecrets } from './queries';

const NAME = /^[A-Za-z][A-Za-z0-9_-]*$/;
const MIN_VALUE = 6;

/** Add a secret, or edit one (its description, who may use it, a new value). */
function SecretDialog({ secret, onClose }: { secret?: Secret; onClose: () => void }) {
  const qc = useQueryClient();
  const { projectOptions, personaOptions } = useScopeOptions({ host: 'The PM (personal workflows)' });
  const [name, setName] = useState(secret?.name ?? '');
  const [description, setDescription] = useState(secret?.description ?? '');
  const [value, setValue] = useState('');
  const [projectIds, setProjectIds] = useState<string[]>(secret?.projects ?? []);
  const [personaIds, setPersonaIds] = useState<string[]>(secret?.personas ?? []);
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => secret
      ? api.patchSecret(secret.id, { description, projects: projectIds, personas: personaIds, ...(value ? { value } : {}) })
      : api.createSecret({ name: name.trim(), description, projects: projectIds, personas: personaIds, value }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: secretKeys.all });
      onClose();
    },
  });
  const nameOk = !!secret || NAME.test(name.trim());
  const valueOk = value === '' ? !!secret : value.length >= MIN_VALUE;
  return (
    <Dialog
      onClose={onClose}
      dismissable={false}
      title={secret ? `Edit ${secret.name}` : 'New secret'}
      description="Agents and workflows use it by name; its value is kept on this machine and never shown in the chat, or to you, again."
      footer={<DialogActions onCancel={onClose} action="Save" disabled={!nameOk || !valueOk} pending={save.isPending} onAction={() => save.mutate()} />}
    >
      <div className="flex flex-col gap-3">
        {!secret && (
          <Field label="Name" hint="Letters, digits, _ and -, starting with a letter: NPM_TOKEN.">
            <Input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
        )}
        <Field label="Description" hint="What it is for (agents see this, never the value).">
          <Textarea aria-label="Description" rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </Field>
        <Field
          label="Value"
          hint={`At least ${MIN_VALUE} characters.${secret ? ' Leave it empty to keep the one saved.' : ''}`}
        >
          <Input aria-label="Value" type="password" autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} />
        </Field>
        <CheckboxPicks label="Projects" hint="Every project when none is ticked." options={projectOptions} value={projectIds} onChange={setProjectIds} />
        <CheckboxPicks label="Agents" hint="Every agent when none is ticked." options={personaOptions} value={personaIds} onChange={setPersonaIds} />
        {save.error && <Alert tone="error">{save.error.message}</Alert>}
      </div>
    </Dialog>
  );
}

/** A secret's uses, newest first. */
function Uses({ id }: { id: string }) {
  const uses = useQuery({ queryKey: secretKeys.uses(id), queryFn: () => api.secretUses(id) });
  if (uses.isLoading) return <Skeleton label="Loading uses" className="h-8" />;
  if (uses.error) return <Alert tone="error">{uses.error.message}</Alert>;
  const list = uses.data ?? [];
  if (!list.length) return <p className="pl-2 text-xs text-muted-foreground">Not used yet.</p>;
  return (
    <ul aria-label="Uses" className="flex w-full flex-col gap-0.5 pl-2 text-xs text-muted-foreground">
      {list.map((u) => (
        <li key={u.id}><RelativeTime time={u.at} /> · {u.by} · <code>{u.how}</code>{u.instance && ` · on ${u.instance}`}</li>
      ))}
    </ul>
  );
}

function Row({ s, names, onEdit, onDelete }: {
  s: Secret;
  names: Map<string, string>;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = useState(false);
  const who = [
    s.projects.length ? s.projects.map((p) => names.get(p) ?? p).join(', ') : 'Every project',
    s.personas.length ? s.personas.map((p) => names.get(p) ?? p).join(', ') : 'every agent',
  ].join(' · ');
  return (
    <div className="flex flex-wrap items-center gap-2 py-1">
      <code className="min-w-0 truncate text-sm font-medium">{s.name}</code>
      <span className="min-w-0 truncate text-xs text-muted-foreground">{who}</span>
      <span className="flex-1" />
      <span className="text-xs text-muted-foreground">
        {s.lastUsedAt !== null ? <>Used <RelativeTime time={s.lastUsedAt} />{s.lastUsedBy && ` by ${s.lastUsedBy}`}</> : <Badge>Not used yet</Badge>}
      </span>
      <Button size="sm" variant="ghost" onClick={() => setOpen((o) => !o)}>{open ? 'Hide uses' : 'Uses'}</Button>
      <Button size="sm" variant="secondary" onClick={onEdit}>Edit</Button>
      <Button size="sm" variant="secondary" onClick={onDelete}>Delete</Button>
      {s.description && <p className="w-full pl-2 text-xs text-muted-foreground">{s.description}</p>}
      {open && <Uses id={s.id} />}
    </div>
  );
}

/** Where the page says values are kept: the store in use, once it's known (PDT-694). */
function storePlace(store: SecretStoreInfo | undefined): string {
  switch (store?.backend) {
    case 'keychain': return "Values are kept in this machine's keychain";
    case 'file': return 'Values are kept in a file in the data folder';
    case 'memory': return 'Values are kept in memory only';
    default: return 'Values are kept on this machine';
  }
}

/** Warnings about where this machine keeps secret values: a file, memory only, or a keychain that didn't answer. */
function StoreNote({ store }: { store: SecretStoreInfo | undefined }) {
  if (!store) return null;
  if (store.backend === 'file') {
    return (
      <Alert tone="warning">
        Secrets: file in the data folder. No system keyring (Secret Service) is reachable on this machine, so values are kept in
        {' '}<code>{store.path}</code>, which only your user can read.
      </Alert>
    );
  }
  if (store.backend === 'memory') {
    return <Alert tone="warning">Secrets aren't saved: no keychain is available, so they're lost when PDT stops.</Alert>;
  }
  return store.problem ? <Alert tone="warning">{store.problem}</Alert> : null;
}

/** On a project's owner: what its remote instances keep for its projects, by name ("on win-box (VAL)"). */
function RemoteSets() {
  const sets = useQuery({ queryKey: secretKeys.remote, queryFn: api.remoteSecrets });
  if (!sets.data?.length) return null;
  return (
    <Section
      label="On remote instances"
      description="Kept on the instance itself for one of your projects, and never sent here: only their names show. Its agents there use these rather than one of yours with the same name."
    >
      {sets.data.map((set) => (
        <div key={`${set.instanceId}-${set.projectId}`} className="flex flex-col gap-0.5 py-1">
          <span className="text-sm font-medium">on {set.instanceName} ({set.projectKey})</span>
          <ul aria-label={`Secrets on ${set.instanceName} for ${set.projectName}`} className="flex flex-col gap-0.5 pl-2">
            {set.secrets.map((n) => (
              <li key={n.name} className="flex flex-wrap items-center gap-2">
                <code className="text-sm">{n.name}</code>
                <span className="text-xs text-muted-foreground">
                  {n.lastUsedAt !== null ? <>Used <RelativeTime time={n.lastUsedAt} />{n.lastUsedBy && ` by ${n.lastUsedBy}`}</> : 'Not used yet'}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </Section>
  );
}

export function SecretsPage() {
  const list = useSecrets();
  const { projects, personas } = useScopeOptions();
  const remote = useRemoteProjects();
  const store = useQuery({ queryKey: secretKeys.store, queryFn: api.secretStore });
  const [editing, setEditing] = useState<{ secret?: Secret } | null>(null);
  const remove = useConfirmDelete({
    remove: (s: Secret) => api.deleteSecret(s.id),
    invalidate: [secretKeys.all],
    confirm: (s) => ({
      title: `Delete ${s.name}?`,
      body: 'Agents and workflows that use it fail until you add one with the same name. Its value is removed from this machine, and its uses are forgotten.',
      action: 'Delete secret',
    }),
  });
  const names = new Map<string, string>([
    ...projects.map((p) => [p.id, p.kind === 'host' ? 'The PM' : p.name] as [string, string]),
    ...personas.map((p) => [p.id, p.name] as [string, string]),
    ...(remote.data ?? []).map((r) => [r.project.id, `${r.project.name} (remote project of ${r.link.ownerName})`] as [string, string]),
  ]);
  return (
    <Page
      crumb="Settings"
      title="Secrets"
      description={`API keys, tokens and passwords agents and workflows use by name, never pasted into the chat. ${storePlace(store.data)} and replaced by [secret:NAME] wherever PDT stores, shows or logs them.`}
    >
      <StoreNote store={store.data} />
      <QueryStatus query={list} what="Secrets" skeleton="h-24">{(items) => (
        <Section
          label="Secrets"
          description="An agent that needs one you haven't added asks for it in your Inbox."
          actions={<Button size="sm" onClick={() => setEditing({})}>Add secret</Button>}
        >
          <Disclosure title="How agents use secrets" panelClassName="mt-1 text-xs text-muted-foreground">
            Agents run a command with secrets in its environment (<code>run_with_secrets</code>) or write a file with them filled
            in (<code>write_secret_file</code>); PDT asks you first. Workflow steps write <code>{'{{secret.NAME}}'}</code> where the
            value goes.
          </Disclosure>
          {items.length
            ? items.map((s) => <Row key={s.id} s={s} names={names} onEdit={() => setEditing({ secret: s })} onDelete={() => void remove.ask(s)} />)
            : <EmptyState size="inline">None yet.</EmptyState>}
        </Section>
      )}</QueryStatus>
      <RemoteSets />
      {remove.error && <Alert tone="error">{remove.error.message}</Alert>}
      {editing && <SecretDialog secret={editing.secret} onClose={() => setEditing(null)} />}
    </Page>
  );
}
