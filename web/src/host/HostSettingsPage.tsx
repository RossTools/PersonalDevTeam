import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Alert, Badge, Button, Input, Link, Page, Section, Select, SettingsRow, Skeleton, StatusDot, Switch, useConfirm, type BadgeTone, type Status } from '@ui';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { BugReportSetting } from '../api/gen/BugReportSetting';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { SessionMode } from '../api/gen/SessionMode';
import type { TokenState } from '../api/gen/TokenState';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';
import { FolderField } from '../common/FolderField';
import { SettingsScreens } from '../common/SettingsScreens';
import { useAction } from '../common/useAction';
import { PermissionModeSelect } from '../conversation/ConversationComposer';
import { BugReportDialog } from './BugReportDialog';

/** The stored token's state as a badge (never the token itself). */
function tokenBadge(present: boolean, t: TokenState | undefined): { tone: BadgeTone; text: string } {
  if (!present || t?.state === 'missing') return { tone: 'neutral', text: 'None' };
  if (t?.state === 'expired') return { tone: 'failed', text: 'Expired or revoked' };
  if (t && 'expiresAt' in t && t.expiresAt !== null) {
    const when = new Date(t.expiresAt).toLocaleDateString();
    return t.state === 'expires_soon' ? { tone: 'waiting', text: `Expires soon (${when})` } : { tone: 'running', text: `Stored · expires ${when}` };
  }
  return { tone: 'running', text: 'Stored' };
}

const updatePolicies: { value: UpdatePolicy; label: string }[] = [
  { value: 'auto_when_idle', label: 'Install automatically when idle' },
  { value: 'notify', label: 'Notify only' },
  { value: 'manual', label: 'Manual (no checks)' },
];
/** How often the host checks for updates, in minutes. */
const checkIntervals: { value: number; label: string }[] = [
  { value: 15, label: 'Every 15 minutes' },
  { value: 30, label: 'Every 30 minutes' },
  { value: 60, label: 'Every hour' },
  { value: 180, label: 'Every 3 hours' },
  { value: 360, label: 'Every 6 hours' },
  { value: 720, label: 'Every 12 hours' },
  { value: 1440, label: 'Once a day' },
];

/** The choices, plus the stored value when it is not one of them (set through the API). */
function intervalOptions(current: number) {
  const all = checkIntervals.some((o) => o.value === current) ? checkIntervals : [...checkIntervals, { value: current, label: `Every ${current} minutes` }].sort((a, b) => a.value - b.value);
  return all.map((o) => ({ value: String(o.value), label: o.label }));
}

const agentChoices: { value: AgentKind; label: string }[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'devin', label: 'Devin' },
];
const viewChoices: { value: SessionMode; label: string }[] = [
  { value: 'conversation', label: 'Conversation' },
  { value: 'terminal', label: 'Terminal' },
];
const credentialPolicies: { value: CredentialPolicy; label: string }[] = [
  { value: 'subscription', label: 'Subscription (remove API keys)' },
  { value: 'pass_through', label: 'Pass through' },
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

/** "Saved" (or nothing) beside a form section's Save. */
function SavedNote({ show }: { show: boolean }) {
  return show ? <span role="status" className="text-xs text-muted-foreground">Saved</span> : null;
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
  const [tone, text]: [BadgeTone, string] = v.remote.active
    ? ['running', 'Active']
    : v.remote.error
      ? ['failed', 'Not active']
      : s.remoteAccess
        ? ['waiting', 'Not active']
        : ['neutral', 'Off'];
  return (
    <Section label="Remote access" description="Reach this host from your other devices over Tailscale.">
      <SettingsRow label="Serve on the tailnet" description="Over HTTPS, through tailscale serve.">
        <SaveNote s={toggle} />
        <Switch aria-label="Remote access" checked={s.remoteAccess} onChange={(on) => toggle.mutate({ remoteAccess: on })} />
      </SettingsRow>
      <SettingsRow
        label="Status"
        description={
          v.remote.active && v.remote.url ? <Link href={v.remote.url}>{v.remote.url}</Link>
            : v.remote.error ? <span className="text-destructive">{v.remote.error}</span>
              : undefined
        }
      >
        <Badge tone={tone}>{text}</Badge>
        <Button size="sm" pending={retrying} onClick={() => void retry()}>Check again</Button>
      </SettingsRow>
      <form onSubmit={submit(() => { if (dirty) owner.mutate({ ownerLogin: login.trim() }); })} className="flex flex-col gap-2">
        <SettingsRow
          label="Owner"
          description={v.ownerFromEnv ? 'Set by PDT_OWNER_LOGIN.' : 'Only this Tailscale user gets in.'}
        >
          <Input aria-label="Owner login" className="w-56" value={v.ownerFromEnv ?? login} disabled={!!v.ownerFromEnv} onChange={(e) => setLogin(e.target.value)} />
          {!v.ownerFromEnv && me && me !== s.ownerLogin && (
            <Button size="sm" onClick={() => owner.mutate({ ownerLogin: me })}>Use {me}</Button>
          )}
          {!v.ownerFromEnv && (
            <Button type="submit" size="sm" variant="primary" aria-label="Save owner" disabled={!dirty} pending={owner.isPending && owner.variables?.ownerLogin === login.trim()}>Save</Button>
          )}
        </SettingsRow>
        {ownerError}
      </form>
    </Section>
  );
}

function Updates({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const policy = useSaveSettings();
  const interval = useSaveSettings();
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
  const [check, checking] = useAction(async () => qc.setQueryData(['host-update'], await api.checkUpdate()));
  const u = update.data;
  const updatesOff = u?.state === 'disabled';
  const badge = tokenBadge(v.tokenPresent, u?.token);
  const checked = u?.lastCheckedAt ? `Last checked ${new Date(u.lastCheckedAt).toLocaleString()}.` : null;
  return (
    <Section label="Updates" description="New releases of PDT, from GitHub.">
      <SettingsRow
        label={`Version ${u?.current ?? '…'}`}
        description={
          updatesOff ? `Updates are off: ${u.disabledReason ?? 'updates apply to the installed app'}.`
            : u?.error ? <span className="text-destructive">{u.error}</span>
              : checked
        }
      >
        {u?.state === 'available' && u.latest && <Badge tone="accent">v{u.latest.version} available</Badge>}
        {!updatesOff && <Button size="sm" pending={checking || u?.state === 'checking'} onClick={() => void check()}>Check for updates</Button>}
      </SettingsRow>
      <SettingsRow label="Install updates">
        <SaveNote s={policy} />
        <Select aria-label="Update policy" className="w-64" options={updatePolicies} value={v.settings.updatePolicy} onChange={(p) => policy.mutate({ updatePolicy: p })} />
      </SettingsRow>
      <SettingsRow label="Check for updates" description={v.settings.updatePolicy === 'manual' ? 'Off while updates are manual.' : undefined}>
        <SaveNote s={interval} />
        <Select
          aria-label="Update check interval"
          className="w-64"
          options={intervalOptions(v.settings.updateCheckMinutes)}
          value={String(v.settings.updateCheckMinutes)}
          disabled={v.settings.updatePolicy === 'manual'}
          onChange={(m) => interval.mutate({ updateCheckMinutes: Number(m) })}
        />
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
        <SettingsRow
          label={<span className="flex items-center gap-2">GitHub token {!v.tokenError && <Badge tone={badge.tone}>{badge.text}</Badge>}</span>}
          description={v.tokenPresent ? 'Kept in the OS keychain. Paste a new one to replace it.' : 'Lets this host read releases. Kept in the OS keychain.'}
        >
          <Input aria-label="GitHub token" type="password" autoComplete="off" className="w-56" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="github_pat_…" />
          <Button type="submit" size="sm" variant="primary" disabled={!secret.trim()} pending={token.isPending}>Save token</Button>
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
        description="Dev instances and other runs started from a branch."
        dirty={portsChanged || maxChanged}
        footer={
          <>
            <SavedNote show={save.saved && !portsChanged && !maxChanged} />
            <Button type="submit" size="sm" variant="primary" aria-label="Save run settings" disabled={!portsChanged && !maxChanged} pending={save.isPending}>Save</Button>
          </>
        }
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
  const agent = useSaveSettings();
  const view = useSaveSettings();
  const ticketMode = useSaveSettings();
  const sessionMode = useSaveSettings();
  const turns = useSaveSettings();
  return (
    <Section label="Agents">
      <SettingsRow label="Default agent" description="Picked first when you start a ticket, and listed first for a new tab and in the palette.">
        <SaveNote s={agent} />
        <Select aria-label="Default agent" className="w-40" options={agentChoices} value={v.settings.defaultAgent} onChange={(a) => agent.mutate({ defaultAgent: a })} />
      </SettingsRow>
      <SettingsRow label="Default view" description="Whether a new agent session is a conversation or the agent's own terminal. You can switch a session later.">
        <SaveNote s={view} />
        <Select aria-label="Default view" className="w-40" options={viewChoices} value={v.settings.defaultSessionMode} onChange={(m) => view.mutate({ defaultSessionMode: m })} />
      </SettingsRow>
      <SettingsRow label="Ticket permissions" description="The permission mode a ticket's Start picks first. In an agent's terminal only Bypass changes anything.">
        <SaveNote s={ticketMode} />
        <PermissionModeSelect
          label="Ticket permissions"
          size="md"
          className="w-48"
          value={v.settings.ticketPermissionMode}
          onChange={(m) => ticketMode.mutate({ ticketPermissionMode: m })}
        />
      </SettingsRow>
      <SettingsRow label="Session permissions" description="The permission mode a new session picks first (the new tab chooser).">
        <SaveNote s={sessionMode} />
        <PermissionModeSelect
          label="Session permissions"
          size="md"
          className="w-48"
          value={v.settings.sessionPermissionMode}
          onChange={(m) => sessionMode.mutate({ sessionPermissionMode: m })}
        />
      </SettingsRow>
      <SettingsRow
        label="Agents running at once"
        description="The most agents running a turn before PDT's own work (workflow steps, workflow messages, auto-start) waits for a slot. What you send is never held."
      >
        <SaveNote s={turns} />
        <Select<string>
          aria-label="Agents running at once"
          className="w-40"
          options={[{ value: '0', label: 'No cap' }, ...[1, 2, 3, 4, 6, 8, 12, 16].map((n) => ({ value: String(n), label: String(n) }))]}
          value={String(v.settings.maxAgentTurns)}
          onChange={(n) => turns.mutate({ maxAgentTurns: Number(n) })}
        />
      </SettingsRow>
      <SettingsRow label="Credentials" description="Subscription removes ANTHROPIC_API_KEY and OPENAI_API_KEY from agents' environment; pass through keeps them (API keys, Bedrock/Vertex, proxies). A project can override it.">
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
        dirty={dirty}
        footer={
          <>
            <SavedNote show={save.saved && !dirty} />
            <Button type="submit" size="sm" variant="primary" aria-label="Save projects folder" disabled={!dirty} pending={save.isPending}>Save</Button>
          </>
        }
      >
        <SettingsRow stacked label="Folder" description="Where “clone by URL” puts new repositories.">
          <FolderField
            aria-label="Projects folder"
            title="Projects folder"
            placeholder={v.projectsDir ?? '~/work'}
            start={v.projectsDir ?? ''}
            value={dir}
            onChange={setDir}
          />
        </SettingsRow>
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
    <Section
      label="Checks"
      description="The programs PDT runs on this host."
      actions={<Button size="sm" pending={refreshing} onClick={() => void refresh()}>Run checks again</Button>}
    >
      {checks.isPending && <Skeleton lines={4} label="Loading checks" />}
      {checks.data && (
        <ul className="flex flex-col gap-1.5">
          {(['claude', 'codex', 'devin', 'git', 'tailscale'] as const).map((k) => {
            const c = checks.data[k];
            // Devin is optional: a host without it is fine.
            const optional = k === 'devin' && !c.found;
            const [status, text]: [Status, string] = c.ok ? ['running', 'OK'] : c.found ? ['waiting', 'needs attention'] : optional ? ['idle', 'not installed (optional)'] : ['failed', 'not found'];
            return (
              <li key={k} className="flex min-w-0 items-center gap-2">
                <StatusDot status={status} label={text} />
                <span className="w-20 shrink-0 font-medium">{k}</span>
                <span className={c.ok || optional ? 'text-muted-foreground' : status === 'failed' ? 'text-destructive' : 'text-waiting'}>{text}</span>
                {!optional && <span className="min-w-0 truncate text-xs text-muted-foreground" title={c.detail}>{c.detail}</span>}
              </li>
            );
          })}
        </ul>
      )}
    </Section>
  );
}

const bugReportChoices: { value: BugReportSetting; label: string }[] = [
  { value: 'auto', label: 'Automatic' },
  { value: 'github', label: 'GitHub issue' },
  { value: 'local', label: 'Local ticket' },
  { value: 'off', label: 'Off' },
];

/** Where Report a bug sends reports (PDT-153), and the form itself. */
function BugReports({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const save = useSaveSettings();
  const preview = useQuery({ queryKey: ['bug-report-preview'], queryFn: api.bugReportPreview });
  const [open, setOpen] = useState(false);
  const p = preview.data;
  const where = !p ? null
    : p.route === 'github' ? `Reports go to ${p.repo} on GitHub as issues, filed with the update token.`
    : p.route === 'local' ? `Reports become Draft bug tickets in ${p.project ? `${p.project.key} · ${p.project.name}` : 'the project whose origin is ' + p.repo}; nothing goes to GitHub.`
    : 'Report a bug is turned off.';
  return (
    <Section
      label="Bug reports"
      description="Where Report a bug sends a report. Automatic files a GitHub issue, except on an instance with PDT's own repository as a project, where it files a local ticket."
      actions={<Button size="sm" onClick={() => setOpen(true)}>Report a bug…</Button>}
    >
      <SettingsRow label="Send reports to" description={where ?? 'Finding where reports go…'}>
        <SaveNote s={save} />
        <Select
          aria-label="Send bug reports to"
          className="w-48"
          options={bugReportChoices}
          value={v.settings.bugReports}
          onChange={(b) => save.mutate({ bugReports: b }, { onSuccess: () => void qc.invalidateQueries({ queryKey: ['bug-report-preview'] }) })}
        />
      </SettingsRow>
      {open && <BugReportDialog onClose={() => setOpen(false)} onOpenSettings={() => setOpen(false)} />}
    </Section>
  );
}

/** Destructive host actions, apart from everything else. Shown only when there is something to remove. */
function DangerZone({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
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
  if (!v.tokenPresent) return null;
  return (
    <Section label="Danger zone" danger>
      <SettingsRow label="Remove the GitHub token" description="Deletes it from the OS keychain. Update checks stop until you save a new one.">
        <Button size="sm" variant="danger" pending={removingToken} onClick={() => void removeToken()}>Remove token…</Button>
      </SettingsRow>
    </Section>
  );
}

/** This host's sections, in page order (their `Section` labels): the phone's list of them and its top bar titles. */
export const HOST_SECTIONS = ['Updates', 'Remote access', 'Agents', 'Runs', 'Projects folder', 'Bug reports', 'Checks', 'Danger zone'] as const;

export interface HostSettingsPageProps {
  /** A phone: one screen per section, the list of them while `section` is null. */
  drill?: boolean;
  section?: string | null;
  onSection?: (slug: string) => void;
}

export function HostSettingsPage({ drill = false, section = null, onSection }: HostSettingsPageProps = {}) {
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
  const nodes: Record<(typeof HOST_SECTIONS)[number], ReactNode> = {
    'Updates': <Updates v={v} />,
    'Remote access': <RemoteAccess v={v} me={me ?? null} />,
    'Agents': <Agents v={v} />,
    'Runs': <Runs v={v} />,
    'Projects folder': <ProjectsFolder v={v} />,
    'Bug reports': <BugReports v={v} />,
    'Checks': <Checks />,
    // Only when there is something to remove.
    'Danger zone': v.tokenPresent ? <DangerZone v={v} /> : null,
  };
  return (
    <SettingsScreens
      title="This host"
      description="Settings for the machine running PDT. Projects have their own pages."
      sections={HOST_SECTIONS.filter((l) => nodes[l] !== null).map((label) => ({ label, node: nodes[label] }))}
      drill={drill}
      section={section}
      onSection={onSection}
    />
  );
}
