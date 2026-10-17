import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { TokenState } from '../api/gen/TokenState';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';

const field = 'rounded-pdt bg-surface border border-border px-2 py-1 text-sm';
const h = 'text-xs uppercase tracking-wider text-muted';

/** Describes the stored token without ever showing it. */
function tokenText(present: boolean, t: TokenState | undefined): string {
  if (!present || t?.state === 'missing') return 'none';
  if (t?.state === 'expired') return 'expired or revoked: replace it';
  if (t && 'expiresAt' in t && t.expiresAt !== null) {
    const when = new Date(t.expiresAt).toLocaleDateString();
    return t.state === 'expires_soon' ? `expires soon (${when}): replace it` : `stored in the OS keychain; expires ${when}`;
  }
  return 'stored in the OS keychain';
}

export function HostSettingsPage() {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks });
  const whoami = useQuery({ queryKey: ['host-whoami'], queryFn: api.whoami });
  const [secret, setSecret] = useState('');
  const [owner, setOwner] = useState('');
  const [ports, setPorts] = useState({ start: '', end: '' });
  const save = useMutation({
    mutationFn: (p: HostSettingsPatch) => api.saveHostSettings(p),
    onSuccess: (v) => qc.setQueryData(['host-settings'], v),
  });
  const token = useMutation({
    mutationFn: (t: string) => api.saveToken(t),
    onSuccess: (u) => {
      qc.setQueryData(['host-update'], u);
      void qc.invalidateQueries({ queryKey: ['host-settings'] });
      setSecret('');
    },
  });
  const removeToken = useMutation({
    mutationFn: api.deleteToken,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['host-settings'] });
      void qc.invalidateQueries({ queryKey: ['host-update'] });
    },
  });
  const retryRemote = useMutation({
    mutationFn: api.refreshRemote,
    onSettled: () => qc.invalidateQueries({ queryKey: ['host-settings'] }),
  });
  const checkUpdate = useMutation({ mutationFn: api.checkUpdate, onSuccess: (u) => qc.setQueryData(['host-update'], u) });
  const refreshChecks = useMutation({ mutationFn: api.refreshChecks, onSuccess: (c) => qc.setQueryData(['host-checks'], c) });
  const v = view.data;
  useEffect(() => {
    if (!v) return;
    setOwner(v.settings.ownerLogin ?? '');
    setPorts({ start: String(v.settings.runPorts.start), end: String(v.settings.runPorts.end) });
  }, [v]);
  if (!v) {
    return view.error
      ? <p role="alert" className="p-4 text-sm text-failed">Host settings could not be loaded: {view.error.message}</p>
      : <p className="p-4 text-sm text-muted">Loading…</p>;
  }
  const s = v.settings;
  const error = save.error ?? token.error ?? removeToken.error ?? checkUpdate.error ?? retryRemote.error;
  const u = update.data;
  // The device being used when it is on the tailnet; otherwise the login this host's Tailscale is signed in as.
  const me = whoami.data?.caller === 'tailnet' && whoami.data.login ? whoami.data.login : v.remote.tailscaleLogin;
  const updatesApply = u?.state === 'disabled';

  return (
    <div className="flex flex-col gap-5 p-4 text-sm overflow-auto max-w-2xl">
      <h2 className="text-base font-semibold">This host</h2>
      {error && <p role="alert" className="text-xs text-failed">{error.message}</p>}

      <section aria-label="Remote access" className="flex flex-col gap-2">
        <h3 className={h}>Remote access</h3>
        <label className="flex items-center gap-2">
          <input type="checkbox" aria-label="Remote access" checked={s.remoteAccess} onChange={(e) => save.mutate({ remoteAccess: e.target.checked })} />
          <span>Serve this host on the tailnet over HTTPS</span>
        </label>
        <p className="text-xs text-muted">
          {v.remote.active && v.remote.url ? <a className="text-accent underline" href={v.remote.url}>{v.remote.url}</a> : v.remote.error ?? (s.remoteAccess ? 'Not active' : 'Off')}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-2">
            <span>Owner</span>
            <input aria-label="Owner login" className={field} value={v.ownerFromEnv ?? owner} disabled={!!v.ownerFromEnv} onChange={(e) => setOwner(e.target.value)} />
          </label>
          {!v.ownerFromEnv && <Button variant="ghost" onClick={() => save.mutate({ ownerLogin: owner })}>Save owner</Button>}
          {!v.ownerFromEnv && me && me !== s.ownerLogin && (
            <Button variant="secondary" onClick={() => save.mutate({ ownerLogin: me })}>Use {me}</Button>
          )}
          {v.ownerFromEnv && <span className="text-xs text-muted">Set by PDT_OWNER_LOGIN</span>}
        </div>
        <div>
          <Button variant="ghost" disabled={retryRemote.isPending} onClick={() => retryRemote.mutate()}>Retry</Button>
        </div>
      </section>

      <section aria-label="Updates" className="flex flex-col gap-2">
        <h3 className={h}>Updates</h3>
        <p className="text-xs text-muted">
          Version {u?.current ?? '…'}
          {updatesApply && ' · Updates apply to the installed app, not this development daemon.'}
          {u?.error && <span className="text-failed"> · {u.error}</span>}
        </p>
        <label className="flex items-center gap-2">
          <span>Policy</span>
          <select aria-label="Update policy" className={field} value={s.updatePolicy} onChange={(e) => save.mutate({ updatePolicy: e.target.value as UpdatePolicy })}>
            <option value="auto_when_idle">Install automatically when idle</option>
            <option value="notify">Notify only</option>
            <option value="manual">Manual (no checks)</option>
          </select>
        </label>
        {v.tokenError ? (
          <div role="alert" className="flex flex-col gap-1 text-xs text-failed">
            <p>Token: cannot be read. {v.tokenError}</p>
            <p className="text-muted">
              A token stored by an older build makes macOS ask before this one may read it. Look for a keychain prompt
              on this Mac and choose Always Allow. If there is none, remove the old item with{' '}
              <code>security delete-generic-password -s dev.personaldevteam -a update-token</code>, restart the daemon
              from the tray, and save the token again here.
            </p>
          </div>
        ) : (
          <p className="text-xs text-muted">Token: {tokenText(v.tokenPresent, u?.token)}</p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <input aria-label="GitHub token" type="password" autoComplete="off" className={field} value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="github_pat_…" />
          <Button variant="secondary" disabled={!secret.trim() || token.isPending} onClick={() => token.mutate(secret.trim())}>Save token</Button>
          {v.tokenPresent && <Button variant="ghost" onClick={() => removeToken.mutate()}>Remove token</Button>}
          {!updatesApply && <Button variant="ghost" disabled={checkUpdate.isPending} onClick={() => checkUpdate.mutate()}>Check now</Button>}
        </div>
      </section>

      <section aria-label="Runs" className="flex flex-col gap-2">
        <h3 className={h}>Runs</h3>
        <div className="flex flex-wrap items-center gap-2">
          <span>Ports</span>
          <input aria-label="First run port" className={`${field} w-24`} value={ports.start} onChange={(e) => setPorts({ ...ports, start: e.target.value })} />
          <span>–</span>
          <input aria-label="Last run port" className={`${field} w-24`} value={ports.end} onChange={(e) => setPorts({ ...ports, end: e.target.value })} />
          <Button variant="ghost" onClick={() => save.mutate({ runPorts: { start: Number(ports.start), end: Number(ports.end) } })}>Save ports</Button>
        </div>
        <label className="flex items-center gap-2">
          <span>Max concurrent runs</span>
          <input key={s.maxConcurrentRuns} aria-label="Max concurrent runs" type="number" min={1} max={32} className={`${field} w-20`} defaultValue={s.maxConcurrentRuns} onBlur={(e) => { if (Number(e.target.value) !== s.maxConcurrentRuns) save.mutate({ maxConcurrentRuns: Number(e.target.value) }); }} />
        </label>
      </section>

      <section aria-label="Agent credentials" className="flex flex-col gap-2">
        <h3 className={h}>Agent credentials</h3>
        <select aria-label="Host agent credentials" className={field} value={s.agentCredentials} onChange={(e) => save.mutate({ agentCredentials: e.target.value as CredentialPolicy })}>
          <option value="subscription">Subscription (remove ANTHROPIC_API_KEY and OPENAI_API_KEY)</option>
          <option value="pass_through">Pass through (API keys, Bedrock/Vertex, proxies)</option>
        </select>
      </section>

      <section aria-label="Checks" className="flex flex-col gap-1">
        <h3 className={h}>Checks</h3>
        {checks.data && (['claude', 'codex', 'git', 'tailscale'] as const).map((k) => (
          <p key={k} className={checks.data[k].ok ? 'text-running' : 'text-failed'}>
            {k}: {checks.data[k].ok ? 'OK' : checks.data[k].found ? 'needs attention' : 'not found'} <span className="text-muted">{checks.data[k].detail}</span>
          </p>
        ))}
        <div>
          <Button variant="ghost" disabled={refreshChecks.isPending} onClick={() => refreshChecks.mutate()}>Run checks again</Button>
        </div>
      </section>
    </div>
  );
}
