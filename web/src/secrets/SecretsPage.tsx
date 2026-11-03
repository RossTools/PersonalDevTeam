// Settings → Secrets (PDT-158): secrets agents and workflows use by name, never pasted into the chat. A value goes to this machine's
// keychain and is never shown again; the list shows each secret's name, who may use it and its last use, and each
// secret's uses on request. It also says where values are kept: the system keyring, or (Linux without a Secret
// Service, PDT-197) a file in the data folder.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, CheckboxPicks, Dialog, DialogActions, EmptyState, Field, Input, Page, RelativeTime, Section, Skeleton, Textarea } from '@ui';
import { api } from '../api/client';
import type { Secret } from '../api/gen/Secret';
import { QueryStatus } from '../common/QueryStatus';
import { useConfirmDelete } from '../common/useConfirmDelete';
import { useScopeOptions } from '../common/useScopeOptions';

const secretsKey = ['secrets'] as const;

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
      void qc.invalidateQueries({ queryKey: secretsKey });
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
      description="Agents and workflows use it by name; its value is kept in this machine's keychain and never shown in the chat, or to you, again."
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
  const uses = useQuery({ queryKey: ['secret-uses', id], queryFn: () => api.secretUses(id) });
  if (uses.isLoading) return <Skeleton label="Loading uses" className="h-8" />;
  if (uses.error) return <Alert tone="error">{uses.error.message}</Alert>;
  const list = uses.data ?? [];
  if (!list.length) return <p className="pl-2 text-xs text-muted-foreground">Not used yet.</p>;
  return (
    <ul aria-label="Uses" className="flex w-full flex-col gap-0.5 pl-2 text-xs text-muted-foreground">
      {list.map((u) => (
        <li key={u.id}><RelativeTime time={u.at} /> · {u.by} · <code>{u.how}</code></li>
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

/** Where this machine keeps secret values. */
function StoreNote() {
  const store = useQuery({ queryKey: [...secretsKey, 'store'], queryFn: api.secretStore });
  if (!store.data) return null;
  if (store.data.backend === 'file') {
    return (
      <Alert tone="warning">
        Secrets: file in the data folder. No system keyring (Secret Service) is reachable on this machine, so values are kept in
        {' '}<code>{store.data.path}</code>, which only your user can read.
      </Alert>
    );
  }
  return (
    <>
      <p className="text-sm text-muted-foreground">
        Secrets: {store.data.backend === 'keychain' ? 'system keyring' : 'in memory (gone when PDT stops)'}
      </p>
      {store.data.problem && <Alert tone="warning">{store.data.problem}</Alert>}
    </>
  );
}

export function SecretsPage() {
  const list = useQuery({ queryKey: secretsKey, queryFn: api.listSecrets });
  const { projects, personas } = useScopeOptions();
  const [editing, setEditing] = useState<{ secret?: Secret } | null>(null);
  const remove = useConfirmDelete({
    remove: (s: Secret) => api.deleteSecret(s.id),
    invalidate: [secretsKey],
    confirm: (s) => ({
      title: `Delete ${s.name}?`,
      body: 'Agents and workflows that use it fail until you add one with the same name. Its value is removed from the keychain, and its uses are forgotten.',
      action: 'Delete secret',
    }),
  });
  const names = new Map<string, string>([
    ...projects.map((p) => [p.id, p.kind === 'host' ? 'The PM' : p.name] as [string, string]),
    ...personas.map((p) => [p.id, p.name] as [string, string]),
  ]);
  return (
    <Page
      title="Secrets"
      description="API keys, tokens and passwords agents and workflows use by name, never pasted into the chat. Values are kept in this machine's keychain and replaced by [secret:NAME] wherever PDT stores, shows or logs them."
    >
      <StoreNote />
      <QueryStatus query={list} what="Secrets" skeleton="h-24">{(items) => (
        <Section
          label="Secrets"
          description="Agents use them with run_with_secrets and write_secret_file, workflows with {{secret.NAME}}; an agent that needs one asks for it in your Inbox."
          actions={<Button size="sm" onClick={() => setEditing({})}>Add secret</Button>}
        >
          {items.length
            ? items.map((s) => <Row key={s.id} s={s} names={names} onEdit={() => setEditing({ secret: s })} onDelete={() => void remove.ask(s)} />)
            : <EmptyState size="inline">None yet.</EmptyState>}
        </Section>
      )}</QueryStatus>
      {remove.error && <Alert tone="error">{remove.error.message}</Alert>}
      {editing && <SecretDialog secret={editing.secret} onClose={() => setEditing(null)} />}
    </Page>
  );
}
