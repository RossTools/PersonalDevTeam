import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Alert, Button, Input, Link, Page, Section, Select, SettingsRow, Skeleton, StatusDot, Switch, useConfirm, type Status } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { TokenState } from '../api/gen/TokenState';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';
import { FolderField } from '../common/FolderField';
import { useAction } from '../common/useAction';

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

const updatePolicies: { value: UpdatePolicy; label: string }[] = [
  { value: 'auto_when_idle', label: 'Install automatically when idle' },
  { value: 'notify', label: 'Notify only' },
  { value: 'manual', label: 'Manual (no checks)' },
];
const credentialPolicies: { value: CredentialPolicy; label: string }[] = [
  { value: 'subscription', label: 'Subscription (remove ANTHROPIC_API_KEY and OPENAI_API_KEY)' },
  { value: 'pass_through', label: 'Pass through (API keys, Bedrock/Vertex, proxies)' },
];

/**
 * Saves a host settings patch. Controls that save immediately (switch, select) show "Saved" for two seconds or their
 * error next to them; forms show the error in an Alert under the form (spec §6.1, §6.3).
 */
function useSaveSettings() {
  const qc = useQueryClient();
  const [saved, setSaved] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const m = useMutation({
    mutationFn: (p: HostSettingsPatch) => api.saveHostSettings(p),
    onMutate: () => setSaved(false),
    onSuccess: (v) => {
      qc.setQueryData(['host-settings'], v);
      setSaved(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setSaved(false), 2000);
    },
  });
  return { ...m, saved };
}

/** "Saved" or the error, next to a control that saves immediately. */
function SaveNote({ s }: { s: ReturnType<typeof useSaveSettings> }) {
  if (s.isError) return <span role="alert" className="text-xs text-destructive">{s.error.message}</span>;
  if (s.saved) return <span role="status" className="text-xs text-muted-foreground">Saved</span>;
  return null;
}

/** A form inside a section: Enter submits, focus moves to its error Alert when the save fails. */
function useFormError(error: Error | null) {
  const id = useId();
  useEffect(() => {
    if (error) document.getElementById(id)?.focus();
  }, [error, id]);
  return error ? <Alert id={id} tabIndex={-1} tone="error">{error.message}</Alert> : null;
}

function submit(fn: () => void) {
  return (e: FormEvent) => {
    e.preventDefault();
    fn();
  };
}

function RemoteAccess({ v, me }: { v: HostSettingsView; me: string | null }) {
  const s = v.settings;
  const qc = useQueryClient();
  const toggle = useSaveSettings();
  const owner = useSaveSettings();
  const [login, setLogin] = useState(s.ownerLogin ?? '');
  useEffect(() => setLogin(s.ownerLogin ?? ''), [s.ownerLogin]);
  const [retry, retrying] = useAction(async () => {
    try {
      await api.refreshRemote();
    } finally {
      void qc.invalidateQueries({ queryKey: ['host-settings'] });
    }
  });
  const ownerError = useFormError(owner.error);
  const dirty = login.trim() !== (s.ownerLogin ?? '');
  const status: ReactNode = v.remote.active && v.remote.url
    ? <Link href={v.remote.url}>{v.remote.url}</Link>
    : v.remote.error ?? (s.remoteAccess ? 'Not active' : 'Off');
  return (
    <Section
      label="Remote access"
      description="Reach this host from your other devices over Tailscale."
      actions={<Button size="sm" variant="ghost" pending={retrying} onClick={() => void retry()}>Retry</Button>}
    >
      <SettingsRow label="Serve this host on the tailnet over HTTPS" description={status}>
        <SaveNote s={toggle} />
        <Switch aria-label="Remote access" checked={s.remoteAccess} onChange={(on) => toggle.mutate({ remoteAccess: on })} />
      </SettingsRow>
      <form onSubmit={submit(() => { if (dirty) owner.mutate({ ownerLogin: login.trim() }); })} className="flex flex-col gap-2">
        <SettingsRow
          label="Owner"
          description={v.ownerFromEnv ? 'Set by PDT_OWNER_LOGIN' : 'Only this Tailscale user gets in.'}
        >
          <Input aria-label="Owner login" className="w-56" value={v.ownerFromEnv ?? login} disabled={!!v.ownerFromEnv} onChange={(e) => setLogin(e.target.value)} />
          {!v.ownerFromEnv && (
            <Button type="submit" aria-label="Save owner" disabled={!dirty} pending={owner.isPending && owner.variables?.ownerLogin === login.trim()}>Save</Button>
          )}
          {!v.ownerFromEnv && me && me !== s.ownerLogin && (
            <Button variant="secondary" onClick={() => owner.mutate({ ownerLogin: me })}>Use {me}</Button>
          )}
        </SettingsRow>
        {ownerError}
      </form>
    </Section>
  );
}

function Updates({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const policy = useSaveSettings();
  const [secret, setSecret] = useState('');
  const token = useMutation({
    mutationFn: (t: string) => api.saveToken(t),
    onSuccess: (u) => {
      qc.setQueryData(['host-update'], u);
      void qc.invalidateQueries({ queryKey: ['host-settings'] });
      setSecret('');
    },
  });
  const tokenError = useFormError(token.error);
  const [removeToken, removingToken] = useAction(async () => {
    const { ok } = await confirm({
      title: 'Remove the GitHub token?',
      body: 'It is deleted from the OS keychain; this host cannot check for updates until you save a new one.',
      action: 'Remove token',
      danger: true,
    });
    if (!ok) return;
    await api.deleteToken();
    void qc.invalidateQueries({ queryKey: ['host-settings'] });
    void qc.invalidateQueries({ queryKey: ['host-update'] });
  });
  const [check, checking] = useAction(async () => qc.setQueryData(['host-update'], await api.checkUpdate()));
  const u = update.data;
  const updatesApply = u?.state === 'disabled';
  return (
    <Section
      label="Updates"
      description={
        <>
          Version {u?.current ?? '…'}
          {updatesApply && ' · Updates apply to the installed app, not this development daemon.'}
          {u?.error && <span className="text-destructive"> · {u.error}</span>}
        </>
      }
      actions={!updatesApply && <Button size="sm" variant="ghost" pending={checking} onClick={() => void check()}>Check now</Button>}
    >
      <SettingsRow label="Policy">
        <SaveNote s={policy} />
        <Select aria-label="Update policy" className="w-64" options={updatePolicies} value={v.settings.updatePolicy} onChange={(p) => policy.mutate({ updatePolicy: p })} />
      </SettingsRow>
      {v.tokenError && (
        <Alert tone="error" title="Token: cannot be read.">
          <p>{v.tokenError}</p>
          <p className="text-muted-foreground">
            A token stored by an older build makes macOS ask before this one may read it. Look for a keychain prompt
            on this Mac and choose Always Allow. If there is none, delete the <code>update-token</code> item in Keychain
            Access, restart the daemon from the tray, and save the token again here.
          </p>
        </Alert>
      )}
      <form onSubmit={submit(() => { if (secret.trim()) token.mutate(secret.trim()); })} className="flex flex-col gap-2">
        <SettingsRow label="Token" description={v.tokenError ? 'A GitHub token for checking releases.' : `A GitHub token for checking releases: ${tokenText(v.tokenPresent, u?.token)}`}>
          <Input aria-label="GitHub token" type="password" autoComplete="off" className="w-56" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="github_pat_…" />
          <Button type="submit" disabled={!secret.trim()} pending={token.isPending}>Save token</Button>
          {v.tokenPresent && <Button variant="ghost" pending={removingToken} onClick={() => void removeToken()}>Remove token</Button>}
        </SettingsRow>
        {tokenError}
      </form>
    </Section>
  );
}

function Runs({ v }: { v: HostSettingsView }) {
  const s = v.settings;
  const save = useSaveSettings();
  const initial = { start: String(s.runPorts.start), end: String(s.runPorts.end), max: String(s.maxConcurrentRuns) };
  const [form, setForm] = useState(initial);
  useEffect(() => setForm({ start: String(s.runPorts.start), end: String(s.runPorts.end), max: String(s.maxConcurrentRuns) }), [s.runPorts.start, s.runPorts.end, s.maxConcurrentRuns]);
  const error = useFormError(save.error);
  const portsChanged = Number(form.start) !== s.runPorts.start || Number(form.end) !== s.runPorts.end;
  const maxChanged = Number(form.max) !== s.maxConcurrentRuns;
  const maxId = useId();
  return (
    <form
      onSubmit={submit(() => save.mutate({
        ...(portsChanged ? { runPorts: { start: Number(form.start), end: Number(form.end) } } : {}),
        ...(maxChanged ? { maxConcurrentRuns: Number(form.max) } : {}),
      }))}
    >
      <Section
        label="Runs"
        actions={<Button type="submit" size="sm" variant="primary" aria-label="Save run settings" disabled={!portsChanged && !maxChanged} pending={save.isPending}>Save</Button>}
      >
        <SettingsRow label="Ports" description="Runs get a port from this range.">
          <Input aria-label="First run port" inputMode="numeric" className="w-20" value={form.start} onChange={(e) => setForm({ ...form, start: e.target.value })} />
          <span className="text-muted-foreground">–</span>
          <Input aria-label="Last run port" inputMode="numeric" className="w-20" value={form.end} onChange={(e) => setForm({ ...form, end: e.target.value })} />
        </SettingsRow>
        <SettingsRow label="Max concurrent runs" htmlFor={maxId}>
          <Input id={maxId} type="number" min={1} max={32} className="w-20" value={form.max} onChange={(e) => setForm({ ...form, max: e.target.value })} />
        </SettingsRow>
        {error}
      </Section>
    </form>
  );
}

function Agents({ v }: { v: HostSettingsView }) {
  const save = useSaveSettings();
  return (
    <Section label="Agent credentials">
      <SettingsRow label="Credentials" description="What agents started on this host may use to sign in.">
        <SaveNote s={save} />
        <Select aria-label="Host agent credentials" className="w-72 max-w-full" options={credentialPolicies} value={v.settings.agentCredentials} onChange={(c) => save.mutate({ agentCredentials: c })} />
      </SettingsRow>
    </Section>
  );
}

function ProjectsFolder({ v }: { v: HostSettingsView }) {
  const saved = v.settings.projectsDir ?? '';
  const save = useSaveSettings();
  const [dir, setDir] = useState(saved);
  useEffect(() => setDir(saved), [saved]);
  const error = useFormError(save.error);
  const dirty = dir.trim() !== saved;
  return (
    <form onSubmit={submit(() => { if (dirty) save.mutate({ projectsDir: dir }); })}>
      <Section
        label="Projects folder"
        description="Where “clone by URL” puts new repositories."
        actions={<Button type="submit" size="sm" variant="primary" aria-label="Save projects folder" disabled={!dirty} pending={save.isPending}>Save</Button>}
      >
        <FolderField
          aria-label="Projects folder"
          title="Projects folder"
          placeholder={v.projectsDir ?? '~/work'}
          start={v.projectsDir ?? ''}
          value={dir}
          onChange={setDir}
        />
        {error}
      </Section>
    </form>
  );
}

function Checks() {
  const qc = useQueryClient();
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks });
  const [refresh, refreshing] = useAction(async () => qc.setQueryData(['host-checks'], await api.refreshChecks()));
  return (
    <Section label="Checks" actions={<Button size="sm" variant="ghost" pending={refreshing} onClick={() => void refresh()}>Run checks again</Button>}>
      {checks.isPending && <Skeleton lines={4} label="Loading checks" />}
      {checks.data && (
        <ul className="flex flex-col gap-1.5">
          {(['claude', 'codex', 'git', 'tailscale'] as const).map((k) => {
            const c = checks.data[k];
            const [status, text]: [Status, string] = c.ok ? ['running', 'OK'] : c.found ? ['waiting', 'needs attention'] : ['failed', 'not found'];
            return (
              <li key={k} className="flex min-w-0 items-center gap-2">
                <StatusDot status={status} label={text} />
                <span className="font-medium">{k}</span>
                <span className={c.ok ? 'text-muted-foreground' : status === 'failed' ? 'text-destructive' : 'text-waiting'}>{text}</span>
                <span className="min-w-0 truncate text-xs text-muted-foreground" title={c.detail}>{c.detail}</span>
              </li>
            );
          })}
        </ul>
      )}
    </Section>
  );
}

export function HostSettingsPage() {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const whoami = useQuery({ queryKey: ['host-whoami'], queryFn: api.whoami });
  const v = view.data;
  if (!v) {
    return (
      <Page title="This host">
        {view.error ? (
          <Alert
            tone="error"
            action={<Button size="sm" onClick={() => void qc.invalidateQueries({ queryKey: ['host-settings'] })}>Retry</Button>}
          >
            Host settings could not be loaded: {view.error.message}
          </Alert>
        ) : (
          <Skeleton label="Loading host settings" className="h-40" />
        )}
      </Page>
    );
  }
  // The device being used when it is on the tailnet; otherwise the login this host's Tailscale is signed in as.
  const me = whoami.data?.caller === 'tailnet' && whoami.data.login ? whoami.data.login : v.remote.tailscaleLogin;
  return (
    <Page title="This host">
      <RemoteAccess v={v} me={me ?? null} />
      <Updates v={v} />
      <Runs v={v} />
      <Agents v={v} />
      <ProjectsFolder v={v} />
      <Checks />
    </Page>
  );
}
