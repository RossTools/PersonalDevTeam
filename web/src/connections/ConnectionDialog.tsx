// Add or edit a connection (Phase 7e): its name, kind, address and secret. The secret goes to the OS keychain and is
// never shown again; leaving it empty when editing keeps the one there is.
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input, Link, Select, Switch, Textarea } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { pluginsKey } from '../plugins/model';
import {
  AUTH_MODES, discordInviteUrl, fieldShown, fieldText, fieldsConfig, hookUrl, kindOf, randomSecret, useConnectionKinds, type ConnectionKind,
} from './kinds';

export const connectionsKey = ['connections'] as const;

const str = (v: unknown) => (typeof v === 'string' ? v : '');

/** Each setting of a kind as text, from a connection's config (a choice starts at its first option). */
function settingsText(k: ConnectionKind | undefined, config: Record<string, unknown> = {}): Record<string, string> {
  return Object.fromEntries((k?.fields ?? []).map((f) => [f.key, fieldText(f, config[f.key]) || (f.type === 'choice' ? f.options?.[0]?.value ?? '' : '')]));
}

interface DialogProps {
  /** Editing this one; else a new one. */
  connection?: Connection;
  /** A new one of this kind (the kind can't be changed). */
  kind?: string;
  onClose: () => void;
  onSaved?: (c: Connection) => void;
  /** Under the title, in place of the usual line (turning a plugin on: why it asks now). */
  description?: string;
  /** The Cancel button's label ("Later"). */
  cancelLabel?: string;
}

/** Opens once the kinds are loaded (the Connections page and pickers have them already). */
export function ConnectionDialog(props: DialogProps) {
  const kinds = useConnectionKinds();
  if (kinds.data) return <ConnectionForm {...props} kinds={kinds.data} />;
  if (!kinds.error) return null;
  return (
    <Dialog open onOpenChange={(o) => { if (!o) props.onClose(); }} title={props.connection ? `Edit ${props.connection.name}` : 'New connection'}
      footer={<Button variant="secondary" onClick={props.onClose}>Cancel</Button>}>
      <Alert tone="error">The kinds of connection could not be loaded: {kinds.error.message}</Alert>
    </Dialog>
  );
}

/** The dialog's form, once the kinds are loaded. */
function ConnectionForm({ connection, kind: fixedKind, onClose, onSaved, description, cancelLabel = 'Cancel', kinds }: DialogProps & { kinds: ConnectionKind[] }) {
  const qc = useQueryClient();
  const [name, setName] = useState(connection?.name ?? '');
  const [kind, setKind] = useState(connection?.kind ?? fixedKind ?? 'discord_webhook');
  const [url, setUrl] = useState(connection?.url ?? '');
  const [auth, setAuth] = useState(str(connection?.config.auth) || 'bearer');
  const [extra, setExtra] = useState(str(connection?.config.username) || str(connection?.config.header) || str(connection?.config.param));
  const [secret, setSecret] = useState('');
  const [settings, setSettings] = useState(() => settingsText(kindOf(kinds, kind), connection?.config));
  const [guildId, setGuildId] = useState(str(connection?.config.guildId));
  const [appId, setAppId] = useState(str(connection?.config.applicationId));
  const [dmUserId, setDmUserId] = useState(str(connection?.config.dmUserId));
  const [dmInbox, setDmInbox] = useState(connection?.config.dmInbox === true);
  const [pmActs, setPmActs] = useState(connection?.config.pmActs === true);
  // A new connection made by an earlier Save whose secret step failed: Save again updates it instead of making a
  // second one with the same name.
  const [made, setMade] = useState<Connection | null>(null);
  const existing = connection ?? made;
  const k = kindOf(kinds, kind);
  const pickKind = (id: string) => { setKind(id); setSettings(settingsText(kindOf(kinds, id))); };
  const askUrl = !!k?.url || (!!k?.urlWhen && settings[k.urlWhen.key] === k.urlWhen.value);
  const extraField = auth === 'basic' ? 'username' : auth === 'header' ? 'header' : auth === 'query' ? 'param' : null;
  const save = useMutation({
    mutationFn: async () => {
      const own: Record<string, unknown> = k?.auth
        ? { auth, ...(extraField ? { [extraField]: extra.trim() } : {}) }
        : k?.discordBot ? { guildId: guildId.trim(), applicationId: appId.trim(), dmUserId: dmUserId.trim(), dmInbox: dmUserId.trim() !== '' && dmInbox }
          : fieldsConfig(k?.fields ?? [], settings);
      if (k?.pmActs) own.pmActs = pmActs;
      // Keeps what PDT put there (a webhook the PM made records its channel), but not a setting the form left empty.
      const kept = Object.fromEntries(Object.entries(connection?.config ?? {}).filter(([key]) => !k?.fields?.some((f) => f.key === key)));
      const config = { ...kept, ...own };
      // An MCP server added here is a workflow's: agents get it once the user ticks Agents in Settings › MCP servers.
      if (kind === 'mcp' && !('agents' in config)) config.agents = { enabled: false };
      const body = { name: name.trim(), kind, url: askUrl ? url.trim() : '', config };
      let c = existing ? await api.updateConnection(existing.id, body) : await api.createConnection(body);
      if (!connection) setMade(c);
      if (secret.trim()) {
        try {
          c = await api.setConnectionSecret(c.id, secret.trim());
        } catch (e) {
          void qc.invalidateQueries({ queryKey: connectionsKey });
          throw new Error(`${body.name} was saved, but not its secret: ${e instanceof Error ? e.message : String(e)}`);
        }
      }
      return c;
    },
    onSuccess: (c) => {
      void qc.invalidateQueries({ queryKey: connectionsKey });
      // A plugin's status follows its connections.
      void qc.invalidateQueries({ queryKey: pluginsKey });
      onSaved?.(c);
      onClose();
    },
  });
  const needsSecret = !existing?.hasSecret && kind !== 'http' && !k?.secretOptional;
  const fieldsFilled = (k?.fields ?? []).every((f) => f.optional || !fieldShown(f, settings) || (settings[f.key] ?? '').trim() !== '');
  const ready = name.trim() !== '' && (!needsSecret || secret.trim() !== '') && (!askUrl || k?.urlOptional || url.trim() !== '')
    && (!k?.discordBot || (/^\d+$/.test(guildId.trim()) && /^\d*$/.test(dmUserId.trim()))) && fieldsFilled;
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      dismissable={false}
      title={connection ? `Edit ${connection.name}` : k ? `Add ${k.label}` : 'New connection'}
      description={description ?? "A service workflows use by name. Its secret is kept in this machine's keychain."}
      footer={<><Button variant="secondary" onClick={onClose}>{cancelLabel}</Button><Button variant="primary" disabled={!ready} pending={save.isPending} onClick={() => save.mutate()}>Save</Button></>}
    >
      <div className="flex flex-col gap-3">
        <Field label="Kind" hint={k?.description}>
          <Select
            className="w-full"
            aria-label="Kind"
            options={kinds.map((x) => ({ value: x.id, label: x.label }))}
            value={kind}
            disabled={!!existing || !!fixedKind}
            onChange={pickKind}
          />
        </Field>
        <Field label="Name" hint="How workflows refer to it, e.g. alerts or home-server.">
          <Input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        {k?.fields?.filter((f) => f.type === 'choice').map((f) => (
          <Field key={f.key} label={f.label} hint={f.hint}>
            <Select className="w-full" aria-label={f.label} options={f.options ?? []} value={settings[f.key] ?? ''}
              onChange={(v) => setSettings((s) => ({ ...s, [f.key]: v }))} />
          </Field>
        ))}
        {askUrl && (
          <Field label="Address" hint={k?.urlHint ?? (k?.auth ? "The service's base URL; a request's path is added to it." : undefined)}>
            <Input aria-label="Address" placeholder={k?.urlPlaceholder ?? 'http://192.168.1.10:7878/api'} value={url} onChange={(e) => setUrl(e.target.value)} />
          </Field>
        )}
        {k?.fields?.filter((f) => f.type !== 'choice' && fieldShown(f, settings)).map((f) => (
          <Field key={f.key} label={f.label} hint={f.hint}>
            {f.type === 'lines' || f.type === 'env'
              ? <Textarea mono aria-label={f.label} rows={3} placeholder={f.placeholder} value={settings[f.key] ?? ''}
                  onChange={(e) => setSettings((s) => ({ ...s, [f.key]: e.target.value }))} />
              : <Input aria-label={f.label} placeholder={f.placeholder} value={settings[f.key] ?? ''}
                  onChange={(e) => setSettings((s) => ({ ...s, [f.key]: e.target.value }))} />}
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
            <Field
              label="Your Discord user ID (optional)"
              hint="To talk to the PM in the bot's DMs: with Developer Mode on, right-click your name → Copy User ID. Only this account's DMs and button presses reach the PM. It needs Message Content Intent on (Developer Portal → Bot → Privileged Gateway Intents)."
            >
              <Input aria-label="Your Discord user ID" inputMode="numeric" value={dmUserId} onChange={(e) => setDmUserId(e.target.value)} />
            </Field>
            {dmUserId.trim() !== '' && (
              <Switch checked={dmInbox} onChange={setDmInbox}>Also send Inbox items (stalled, needs input) as DMs</Switch>
            )}
          </>
        )}
        {k?.pmActs && <Switch checked={pmActs} onChange={setPmActs}>{k.pmActs}</Switch>}
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
