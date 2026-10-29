import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Alert, Badge, Button, Input, Link, Section, SettingsRow, Skeleton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { ForgeAccountView } from '../api/gen/ForgeAccountView';
import type { ForgeLoginView } from '../api/gen/ForgeLoginView';
import type { ForgeTokenCheck } from '../api/gen/ForgeTokenCheck';

/**
 * A forge's logins, on its plugin page (Settings › Plugins › GitHub; PDT-177, PDT-228, PDT-244): one account per host
 * (github.com and a GitHub Enterprise Server at once), each kept in the OS keychain. Add one with a pasted token for
 * any host (Test shows who it signs in as), or, where the forge has it, the device flow (enter a code on the forge).
 */
export function ForgeLogin({ kind, label }: { kind: string; label: string }) {
  const qc = useQueryClient();
  const key = ['forge-login', kind];
  const view = useQuery({
    queryKey: key,
    queryFn: () => api.forgeLogin(kind),
    // While a code waits to be entered, look often: PDT signs in as soon as the forge says so.
    refetchInterval: (q) => (q.state.data?.pending ? 2000 : false),
  });
  const set = (v: ForgeLoginView) => qc.setQueryData(key, v);
  const [token, setToken] = useState('');
  const [apiUrl, setApiUrl] = useState<string | null>(null);
  const [clientId, setClientId] = useState('');
  const [tested, setTested] = useState<ForgeTokenCheck | null>(null);
  const v = view.data;
  const url = (apiUrl ?? v?.defaultApiUrl ?? '').trim();
  const isPublic = v?.defaultApiUrl != null && url.replace(/\/+$/, '') === v.defaultApiUrl;
  // The public host's API is the server's default: sent only when it is another host.
  const given = isPublic || !url ? undefined : url;
  const device = useMutation({
    mutationFn: (b: { apiUrl?: string; clientId?: string }) => api.forgeLoginDevice(kind, b),
    onSuccess: set,
  });
  const test = useMutation({ mutationFn: () => api.forgeTestToken(kind, token.trim(), given), onSuccess: setTested });
  const paste = useMutation({
    mutationFn: () => api.forgeLoginToken(kind, token.trim(), given),
    onSuccess: (r) => { set(r); setToken(''); setTested(null); },
  });
  const cancel = useMutation({ mutationFn: () => api.forgeCancelDevice(kind), onSuccess: set });
  const disconnect = useMutation({ mutationFn: (host: string) => api.forgeDisconnect(kind, host), onSuccess: set });
  const confirm = useConfirm();
  const description = `The logins PDT reads pull requests, CI and issues with, and merges and comments as: one for each ${label} host.`;
  if (!v) {
    return (
      <Section label={`${label} logins`} description={description}>
        {view.error ? <Alert tone="error">The {label} logins could not be read: {view.error.message}</Alert> : <Skeleton label={`Loading the ${label} logins`} className="h-16" />}
      </Section>
    );
  }

  const onDisconnect = async (a: ForgeAccountView) => {
    const { ok } = await confirm({
      title: `Disconnect ${label} on ${a.host}?`,
      body: `PDT stops following pull requests and CI, merging and commenting on ${a.host} until you connect again. The login leaves the keychain; revoke it on ${label} too if you like.`,
      action: 'Disconnect',
      danger: true,
    });
    if (ok) disconnect.mutate(a.host);
  };
  const onPaste = (e: FormEvent) => {
    e.preventDefault();
    if (token.trim()) paste.mutate();
  };
  const publicHost = v.defaultApiUrl ? v.accounts.find((a) => a.apiUrl === v.defaultApiUrl) : undefined;
  // The device flow: on the public host with PDT's own OAuth App, elsewhere with that host's.
  const canDevice = v.hasDeviceFlow && (isPublic ? v.deviceFlow : clientId.trim() !== '');
  const error = device.error ?? paste.error ?? disconnect.error;

  return (
    <Section label={`${label} logins`} description={description}>
      {v.accounts.length === 0 && (
        <Alert tone="warning" title={`${label} is not connected.`}>
          Until it is, PDT can’t follow pull requests or CI, merge, or let agents read CI logs and comment.
        </Alert>
      )}
      {v.accounts.map((a) => (
        <div key={a.host} className="flex flex-col gap-2">
          {a.rejectedAt != null && (
            <Alert tone="error" title={`${label} on ${a.host} no longer accepts this login.`}>
              The token was revoked or expired. PDT has stopped calling {a.host} until you connect again: reconnect
              below, or paste a new token for it.
            </Alert>
          )}
          <SettingsRow
            label={<span className="flex items-center gap-2">Connected <Badge tone="accent">{a.login}</Badge></span>}
            description={`On ${a.host}.${expiry(a)}`}
          >
            <Button size="sm" variant="danger" aria-label={`Disconnect ${a.host}`} pending={disconnect.isPending && disconnect.variables === a.host} onClick={() => void onDisconnect(a)}>Disconnect…</Button>
          </SettingsRow>
        </div>
      ))}
      {v.pending ? (
        <SettingsRow
          label={<span>Enter <code className="font-mono text-base font-semibold">{v.pending.userCode}</code> on {label}</span>}
          description={<>Open <Link href={v.pending.verificationUri} external>{v.pending.verificationUri}</Link>, enter the code and authorize PDT. This page updates once you have.</>}
        >
          <Button size="sm" pending={cancel.isPending} onClick={() => cancel.mutate()}>Cancel</Button>
        </SettingsRow>
      ) : (
        canDevice && (
          <SettingsRow
            label={isPublic && publicHost ? 'Connect again' : `Connect ${label}`}
            description={`Sign in on ${isPublic ? label : url} with a code. PDT asks for the ${v.scopes} scope.`}
          >
            <Button
              size="sm"
              variant={publicHost && publicHost.rejectedAt == null && isPublic ? 'secondary' : 'primary'}
              pending={device.isPending}
              onClick={() => device.mutate(isPublic ? {} : { apiUrl: url, clientId: clientId.trim() })}
            >
              {isPublic && publicHost?.rejectedAt != null ? 'Reconnect' : `Connect ${label}`}
            </Button>
          </SettingsRow>
        )
      )}
      {v.error && <Alert tone="error" title="The login failed.">{v.error}</Alert>}
      <form onSubmit={onPaste} className="flex flex-col gap-2">
        <SettingsRow
          label="Host"
          description={v.defaultApiUrl ? `Its API URL: ${v.defaultApiUrl} for the public host, or your own server’s, e.g. ${v.apiUrlExample}.` : `Its API URL, e.g. ${v.apiUrlExample}.`}
        >
          <Input aria-label={`${label} API URL`} className="w-72" value={apiUrl ?? v.defaultApiUrl ?? ''} onChange={(e) => { setApiUrl(e.target.value); setTested(null); }} placeholder={v.apiUrlExample} />
        </SettingsRow>
        <SettingsRow
          label="Or paste a token"
          description={<>A token with the <code>{v.scopes}</code> scope. {v.tokenHelp} Kept in the OS keychain.</>}
        >
          <Input aria-label={`${label} login token`} type="password" autoComplete="off" className="w-56" value={token} onChange={(e) => { setToken(e.target.value); setTested(null); }} />
          <Button size="sm" disabled={!token.trim()} pending={test.isPending} onClick={() => test.mutate()}>Test</Button>
          <Button type="submit" size="sm" disabled={!token.trim()} pending={paste.isPending}>Save login token</Button>
        </SettingsRow>
        {tested && (
          <Alert tone="info" title={`Signs in as ${tested.login} on ${tested.host}.`}>
            {tested.scopes.length ? `Its scopes: ${tested.scopes.join(', ')}.` : `${label} names no scopes for it (a fine-grained token has permissions instead).`}
          </Alert>
        )}
        {test.error && <Alert tone="error">{test.error.message}</Alert>}
      </form>
      {v.hasDeviceFlow && !isPublic && (
        <SettingsRow label="OAuth App client ID" description="To connect with a code there: an OAuth App registered on that host, with device flow on. PDT’s own works on the public host only.">
          <Input aria-label="OAuth App client ID" className="w-72" value={clientId} onChange={(e) => setClientId(e.target.value)} />
        </SettingsRow>
      )}
      {error && <Alert tone="error">{error.message}</Alert>}
    </Section>
  );
}

/** When an account's token expires (PDT-260), if it does: one PDT renews, or one to replace in time. */
function expiry(a: { expiresAt: number | null; renews: boolean }): string {
  if (a.expiresAt == null) return '';
  const at = new Date(a.expiresAt).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
  return a.renews
    ? ` Its token expires ${at}; PDT renews it before then.`
    : ` Its token expires ${at}: connect again before then.`;
}
