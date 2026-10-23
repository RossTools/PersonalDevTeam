// Add or edit a connection (Phase 7e): its name, kind, address and secret. The secret goes to the OS keychain and is
// never shown again; leaving it empty when editing keeps the one there is.
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input, Link, Select, Switch } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { AUTH_MODES, CONNECTION_KINDS, discordInviteUrl, hookUrl, kindOf, randomSecret } from './kinds';

export const connectionsKey = ['connections'] as const;

const str = (v: unknown) => (typeof v === 'string' ? v : '');

export function ConnectionDialog({ connection, kind: fixedKind, onClose, onSaved }: {
  /** Editing this one; else a new one. */
  connection?: Connection;
  /** A new one of this kind (the kind can't be changed). */
  kind?: string;
  onClose: () => void;
  onSaved?: (c: Connection) => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState(connection?.name ?? '');
  const [kind, setKind] = useState(connection?.kind ?? fixedKind ?? 'discord_webhook');
  const [url, setUrl] = useState(connection?.url ?? '');
  const [auth, setAuth] = useState(str(connection?.config.auth) || 'bearer');
  const [extra, setExtra] = useState(str(connection?.config.username) || str(connection?.config.header) || str(connection?.config.param));
  const [secret, setSecret] = useState('');
  const [fields, setFields] = useState<Record<string, string>>(() =>
    Object.fromEntries(Object.entries(connection?.config ?? {}).map(([key, v]) => [key, str(v)])));
  const [guildId, setGuildId] = useState(str(connection?.config.guildId));
  const [appId, setAppId] = useState(str(connection?.config.applicationId));
  const [pmActs, setPmActs] = useState(connection?.config.pmActs === true);
  const k = kindOf(kind);
  const extraField = auth === 'basic' ? 'username' : auth === 'header' ? 'header' : auth === 'query' ? 'param' : null;
  const save = useMutation({
    mutationFn: async () => {
      const own: Record<string, unknown> = k?.auth
        ? { auth, ...(extraField ? { [extraField]: extra.trim() } : {}) }
        : k?.discordBot ? { guildId: guildId.trim(), applicationId: appId.trim(), pmActs } : {};
      for (const f of k?.fields ?? []) own[f.key] = (fields[f.key] ?? '').trim();
      // Keeps what PDT put there (a webhook the PM made records its channel).
      const config = { ...connection?.config, ...own };
      const body = { name: name.trim(), kind, url: k?.url ? url.trim() : '', config };
      let c = connection ? await api.updateConnection(connection.id, body) : await api.createConnection(body);
      if (secret.trim()) c = await api.setConnectionSecret(c.id, secret.trim());
      return c;
    },
    onSuccess: (c) => {
      void qc.invalidateQueries({ queryKey: connectionsKey });
      onSaved?.(c);
      onClose();
    },
  });
  const needsSecret = !connection?.hasSecret && kind !== 'http';
  const fieldsFilled = (k?.fields ?? []).every((f) => (fields[f.key] ?? '').trim() !== '');
  const ready = name.trim() !== '' && (!needsSecret || secret.trim() !== '') && (!k?.url || k.urlOptional || url.trim() !== '')
    && (!k?.discordBot || /^\d+$/.test(guildId.trim())) && fieldsFilled;
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      dismissable={false}
      title={connection ? `Edit ${connection.name}` : 'New connection'}
      description="A service workflows use by name. Its secret is kept in this machine's keychain."
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!ready} pending={save.isPending} onClick={() => save.mutate()}>Save</Button></>}
    >
      <div className="flex flex-col gap-3">
        <Field label="Kind" hint={k?.description}>
          <Select
            className="w-full"
            aria-label="Kind"
            options={CONNECTION_KINDS.map((x) => ({ value: x.id, label: x.label }))}
            value={kind}
            disabled={!!connection || !!fixedKind}
            onChange={setKind}
          />
        </Field>
        <Field label="Name" hint="How workflows refer to it, e.g. alerts or home-server.">
          <Input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        {k?.url && (
          <Field label="Address" hint={k.urlHint ?? "The service's base URL; a request's path is added to it."}>
            <Input aria-label="Address" placeholder={k.urlPlaceholder ?? 'http://192.168.1.10:7878/api'} value={url} onChange={(e) => setUrl(e.target.value)} />
          </Field>
        )}
        {k?.fields?.map((f) => (
          <Field key={f.key} label={f.label} hint={f.hint}>
            <Input aria-label={f.label} placeholder={f.placeholder} value={fields[f.key] ?? ''} onChange={(e) => setFields({ ...fields, [f.key]: e.target.value })} />
          </Field>
        ))}
        {k?.discordBot && (
          <>
            <Field label="Server ID" hint="In Discord, turn on Developer Mode (Settings → Advanced), then right-click the server → Copy Server ID.">
              <Input aria-label="Server ID" inputMode="numeric" value={guildId} onChange={(e) => setGuildId(e.target.value)} />
            </Field>
            <Field label="Application ID" hint="Developer Portal → your application → General Information. Used only to make the invite link.">
              <Input aria-label="Application ID" inputMode="numeric" value={appId} onChange={(e) => setAppId(e.target.value)} />
            </Field>
            {/^\d+$/.test(appId.trim()) && (
              <Alert tone="info" title="Invite the bot">
                <Link href={discordInviteUrl(appId.trim())} external className="break-all text-xs">{discordInviteUrl(appId.trim())}</Link>
                <div className="text-xs text-muted-foreground">Open it, pick the server, and keep the permissions it asks for.</div>
              </Alert>
            )}
            <Switch checked={pmActs} onChange={setPmActs}>The PM changes this server without asking</Switch>
          </>
        )}
        {k?.auth && (
          <Field label="Send it as">
            <Select className="w-full" aria-label="Send it as" options={AUTH_MODES} value={auth} onChange={setAuth} />
          </Field>
        )}
        {k?.auth && extraField && (
          <Field label={extraField === 'username' ? 'User' : extraField === 'header' ? 'Header name' : 'Parameter name'}>
            <Input aria-label={extraField} placeholder={extraField === 'header' ? 'X-Api-Key' : undefined} value={extra} onChange={(e) => setExtra(e.target.value)} />
          </Field>
        )}
        {(!k?.auth || auth !== 'none') && (
          <Field
            label={k?.secretLabel ?? 'Secret'}
            hint={<>{k?.secretHint}{connection?.hasSecret ? ' One is saved: leave this empty to keep it.' : ''}</>}
          >
            <div className="flex items-center gap-1">
              <Input aria-label={k?.secretLabel ?? 'Secret'} type={k?.generate ? 'text' : 'password'} className="flex-1" value={secret} onChange={(e) => setSecret(e.target.value)} />
              {k?.generate && <Button size="sm" variant="secondary" onClick={() => setSecret(randomSecret())}>Generate</Button>}
            </div>
          </Field>
        )}
        {kind === 'webhook' && name.trim() && (
          <Alert tone="info" title="The sender posts to">
            <code className="break-all text-xs">{hookUrl(name.trim())}</code>
            <div className="text-xs text-muted-foreground">Copy the secret now: it isn't shown again.</div>
          </Alert>
        )}
        {save.error && <Alert tone="error">{save.error.message}</Alert>}
      </div>
    </Dialog>
  );
}
