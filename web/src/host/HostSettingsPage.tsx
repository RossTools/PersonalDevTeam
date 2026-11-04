import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Alert, Badge, Button, Disclosure, EmptyState, Input, Link, Page, Section, Select, SettingsRow, Skeleton, StatusDot, Switch, useConfirm, useThemePref, type BadgeTone, type Status, type ThemePref } from '@ui';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { BugReportSetting } from '../api/gen/BugReportSetting';
import type { CheckResult } from '../api/gen/CheckResult';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { SessionMode } from '../api/gen/SessionMode';
import type { TokenState } from '../api/gen/TokenState';
import type { UpdateChannel } from '../api/gen/UpdateChannel';
import type { UpdateNotice } from '../api/gen/UpdateNotice';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';
import type { NetworkSettings } from '../api/gen/NetworkSettings';
import { AutoSaveText, SaveStatus, useSaveState } from '../common/AutoSave';
import { FolderField } from '../common/FolderField';
import { HOST_SECTIONS } from '../common/settingsSections';
import { SettingsScreens } from '../common/SettingsScreens';
import { useAction } from '../common/useAction';
import { DEFAULT_MODEL, modelChoices, PermissionModeSelect } from '../conversation/ConversationComposer';
import { navigate } from '../shell/router';
import { useInstance } from '../shell/useInstance';
import { useAgents, useAgentsOff } from '../common/agents';
import { AcpAgents } from './AcpAgents';
import { agentsOff, AgentSwitch, offNote, switchBlocked } from './AgentSwitch';
import { StartAtLogin } from './StartAtLogin';
import { BugReportDialog } from './BugReportDialog';
import { Storage } from './StorageSettings';
import { InstallUpdates } from './InstallUpdates';
import { StuckAgents } from './StuckAgents';
import { PmCheckpointSettings } from '../pm/PmCheckpointSettings';
import { documentKeys, useDocumentProviders } from '../documents/queries';
import { hostKeys, useHostSettings, setHostSettings, useHostChecks, useAgentCapabilities } from './queries';
import { LoadError } from '../common/QueryStatus';
import { STEPS } from '../setup/SetupPage';

/** The stored token's state as a badge (never the token itself). */
function tokenBadge(present: boolean, t: TokenState | undefined): { tone: BadgeTone; text: string } {
  if (!present || t?.state === 'missing') return { tone: 'neutral', text: 'None' };
  if (t?.state === 'expired') return { tone: 'failed', text: 'Rejected, ignored' };
  if (t && 'expiresAt' in t && t.expiresAt !== null) {
    const when = new Date(t.expiresAt).toLocaleDateString();
    return t.state === 'expires_soon' ? { tone: 'waiting', text: `Expires soon (${when})` } : { tone: 'running', text: `Stored · expires ${when}` };
  }
  return { tone: 'running', text: 'Stored' };
}

/** When GitHub's rate limit resets, as "after 14:05" (empty when GitHub didn't say). */
function resetNote(retryAt: number | null | undefined): string {
  return retryAt ? ` (after ${new Date(retryAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })})` : '';
}

/** What a check the user asked for found, said in the Version row; `fixToken` points at the token field. */
function CheckFound({ r, retryAt, fixToken }: { r: CheckResult; retryAt: number | null | undefined; fixToken: () => void }) {
  if (r.kind === 'upToDate') return <>You're up to date (v{r.version}).</>;
  if (r.kind === 'available') return <>v{r.version} is available: install it from the banner at the top of the page.</>;
  if (r.reason === 'rateLimited') return <>GitHub's limit on checks is used up for now; PDT checks again once it resets{resetNote(retryAt)}.</>;
  return (
    <span className="text-destructive">
      Couldn't check: {r.message}.{r.reason === 'tokenRejected' && <> <Link onClick={fixToken}>Replace the token</Link></>}
    </span>
  );
}

/** What the last check wants the user to know without having failed: a rejected token, a rate limit. */
function CheckNotice({ notice, retryAt, fixToken }: { notice: UpdateNotice; retryAt: number | null; fixToken: () => void }) {
  if (notice.kind === 'rateLimited') return <>GitHub's limit on checks is used up for now; PDT checks again once it resets{resetNote(retryAt)}.</>;
  return (
    <span className="text-waiting">
      {notice.message[0].toUpperCase() + notice.message.slice(1)}. <Link onClick={fixToken}>Replace the token</Link>
    </span>
  );
}

const updatePolicies: { value: UpdatePolicy; label: string }[] = [
  { value: 'auto_when_idle', label: 'Install automatically when idle' },
  { value: 'notify', label: 'Notify only' },
  { value: 'manual', label: 'Manual (no checks)' },
];
const updateChannels: { value: UpdateChannel; label: string }[] = [
  { value: 'stable', label: 'Stable releases' },
  { value: 'nightly', label: 'Nightly builds too' },
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
 * Saves a host settings patch. Every setting saves as it changes (PDT-231): a control that saves at once shows
 * "Saved" for a moment or its error beside it (`SaveNote`).
 */
function useSaveSettings() {
  const qc = useQueryClient();
  const [state, track] = useSaveState();
  const m = useMutation({
    meta: { errorShown: true },
    mutationFn: (p: HostSettingsPatch) => api.saveHostSettings(p),
    onSuccess: (v) => setHostSettings(qc, v),
  });
  return { ...m, state, save: (p: HostSettingsPatch) => track(m.mutateAsync(p)), saveText: (p: HostSettingsPatch) => m.mutateAsync(p) };
}

/** "Saved" or the error, next to a control that saves immediately. */
function SaveNote({ s }: { s: ReturnType<typeof useSaveSettings> }) {
  return <SaveStatus state={s.state} />;
}

/** A form inside a section (secrets and tokens keep an explicit Save): focus moves to its error Alert when it fails. */
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

/** "env" beside a setting the environment sets: read-only here. */
function EnvBadge({ name }: { name: string }) {
  return <Badge title={`Set by ${name}`}>env</Badge>;
}

function RemoteAccess({ v, me }: { v: HostSettingsView; me: string | null }) {
  const s = v.settings;
  const qc = useQueryClient();
  const toggle = useSaveSettings();
  const owner = useSaveSettings();
  const [retry, retrying] = useAction(async () => {
    try {
      await api.refreshRemote();
    } finally {
      void qc.invalidateQueries({ queryKey: hostKeys.settings });
    }
  });
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
        <Switch aria-label="Remote access" checked={s.remoteAccess} onChange={(on) => void toggle.save({ remoteAccess: on })} />
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
      <SettingsRow
        label="Owner"
        description={v.ownerFromEnv ? 'Set by PDT_OWNER_LOGIN, which wins over this setting.' : 'Only this Tailscale user gets in.'}
      >
        {v.ownerFromEnv ? (
          <>
            <EnvBadge name="PDT_OWNER_LOGIN" />
            <Input aria-label="Owner login" className="w-56" value={v.ownerFromEnv} disabled />
          </>
        ) : (
          <>
            <SaveNote s={owner} />
            <AutoSaveText aria-label="Owner login" className="w-56" value={s.ownerLogin ?? ''} onSave={(ownerLogin) => owner.saveText({ ownerLogin })} />
            {me && me !== s.ownerLogin && <Button size="sm" onClick={() => void owner.save({ ownerLogin: me })}>Use {me}</Button>}
          </>
        )}
      </SettingsRow>
    </Section>
  );
}

function Updates({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const update = useQuery({ queryKey: hostKeys.update, queryFn: api.updateStatus });
  const policy = useSaveSettings();
  const channel = useSaveSettings();
  const interval = useSaveSettings();
  const repoSave = useSaveSettings();
  const [secret, setSecret] = useState('');
  const token = useMutation({
    meta: { errorShown: true },
    mutationFn: (t: string) => api.saveToken(t),
    onSuccess: (u) => {
      qc.setQueryData(hostKeys.update, u);
      void qc.invalidateQueries({ queryKey: hostKeys.settings });
      setSecret('');
    },
  });
  const tokenError = useFormError(token.error);
  const tokenInput = useRef<HTMLInputElement>(null);
  // The token is only for a private repository or a fork: folded away unless one is stored.
  const [advanced, setAdvanced] = useState(v.tokenPresent || !!v.tokenError);
  // The last check the button asked for, shown until another check (here or the background one) answers.
  const [found, setFound] = useState<{ result: CheckResult; at: number | null } | null>(null);
  const [check, checking] = useAction(async () => {
    const { result, ...status } = await api.checkUpdate();
    qc.setQueryData(hostKeys.update, status);
    return { result, at: status.lastCheckedAt };
  });
  const checkNow = async () => {
    const r = await check();
    if (r) setFound(r);
  };
  const u = update.data;
  const shownFound = found && u && found.at === u.lastCheckedAt ? found.result : null;
  // Focusing scrolls the field into view (once Advanced is open).
  const fixToken = () => {
    setAdvanced(true);
    requestAnimationFrame(() => tokenInput.current?.focus());
  };
  const updatesOff = u?.state === 'disabled';
  const badge = tokenBadge(v.tokenPresent, u?.token);
  const checked = u?.lastCheckedAt ? `Last checked ${new Date(u.lastCheckedAt).toLocaleString()}.` : null;
  const where = u?.releaseRepo;
  const fromEnv = where?.overriddenBy === 'environment';
  const repoNote = fromEnv ? `Set by PDT_RELEASE_REPO: ${where.repo}.`
    : where?.movedTo ? `GitHub says it moved (to ${where.movedTo}); PDT follows it.`
      : 'Where updates come from and bug reports go. PDT follows GitHub\'s redirects when it is renamed or moved: set another one only if it can\'t find it.';
  // Never a downgrade: back on stable, a nightly build stays until a newer stable release exists.
  const channelNote = v.settings.updateChannel === 'nightly'
    ? 'A build of main every night it changed (vX.Y.Z-nightly.YYYYMMDD), and every stable release. They can carry database migrations no stable release has yet, so this host can\'t go back to an older stable release.'
    : u?.currentNightly
      ? `This host stays on the nightly build v${u.current} until a newer stable release exists.`
      : 'Full releases only.';
  const saveRepo = async (r: string) => {
    await repoSave.saveText({ releaseRepo: r });
    void qc.invalidateQueries({ queryKey: hostKeys.update });
  };
  return (
    <Section label="Updates" description="New releases of PDT, from GitHub.">
      <SettingsRow
        label={<span className="flex items-center gap-2">Version {u?.current ?? '…'} {u?.currentNightly && <Badge>Nightly</Badge>}</span>}
        description={
          updatesOff ? `Updates are off: ${u.disabledReason ?? 'updates apply to the installed app'}.`
            : shownFound ? <span role="status"><CheckFound r={shownFound} retryAt={u?.retryAt} fixToken={fixToken} /> {shownFound.kind !== 'cannotCheck' && checked}</span>
              : u?.error ? <span className="text-destructive">{u.error}</span>
              : u?.notice ? <span><CheckNotice notice={u.notice} retryAt={u.retryAt} fixToken={fixToken} /> {checked}</span>
              : checked
        }
      >
        {u?.state === 'available' && u.latest && <Badge tone="accent">v{u.latest.version} available{u.latest.nightly ? ' (nightly)' : ''}</Badge>}
        {!updatesOff && <Button size="sm" pending={checking || u?.state === 'checking'} onClick={() => void checkNow()}>Check for updates</Button>}
      </SettingsRow>
      <SettingsRow label="Install updates">
        <SaveNote s={policy} />
        <Select aria-label="Update policy" className="w-64" options={updatePolicies} value={v.settings.updatePolicy} onChange={(p) => void policy.save({ updatePolicy: p })} />
      </SettingsRow>
      <SettingsRow label="Update channel" description={channelNote}>
        <SaveNote s={channel} />
        <Select
          aria-label="Update channel"
          className="w-64"
          options={updateChannels}
          value={v.settings.updateChannel}
          // Checks again at once, so an update the other channel offered isn't left showing.
          onChange={(c) => void channel.save({ updateChannel: c }).then((ok) => { if (ok && !updatesOff) void check(); })}
        />
      </SettingsRow>
      <SettingsRow label="Check for updates" description={v.settings.updatePolicy === 'manual' ? 'Off while updates are manual.' : undefined}>
        <SaveNote s={interval} />
        <Select
          aria-label="Update check interval"
          className="w-64"
          options={intervalOptions(v.settings.updateCheckMinutes)}
          value={String(v.settings.updateCheckMinutes)}
          disabled={v.settings.updatePolicy === 'manual'}
          onChange={(m) => void interval.save({ updateCheckMinutes: Number(m) })}
        />
      </SettingsRow>
      <SettingsRow label="Release repository" description={repoNote}>
        {fromEnv ? (
          <>
            <EnvBadge name="PDT_RELEASE_REPO" />
            <Input aria-label="Release repository" className="w-56" value={where.repo} disabled />
          </>
        ) : (
          <>
            <AutoSaveText
              aria-label="Release repository"
              className="w-56"
              value={v.settings.releaseRepo ?? ''}
              placeholder={where?.overriddenBy ? undefined : where?.repo}
              onSave={saveRepo}
            />
            {v.settings.releaseRepo && <Button size="sm" onClick={() => void saveRepo('')}>Use the built-in one</Button>}
          </>
        )}
      </SettingsRow>
      <Disclosure title="Advanced" open={advanced} onOpenChange={setAdvanced} className="text-sm" panelClassName="mt-2 flex flex-col gap-2 text-foreground">
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
            description={u?.token.state === 'expired' ? 'GitHub rejected it, so updates go on without it. Paste a new one, or remove it under Danger zone.'
              : v.tokenPresent ? 'Only needed for a private repository or a fork. Kept in the OS keychain; paste a new one to replace it.'
                : 'Only needed for a private repository or a fork. Kept in the OS keychain. Report a bug uses your GitHub login, not this.'}
          >
            <Input ref={tokenInput} aria-label="GitHub token" type="password" autoComplete="off" className="w-56" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="github_pat_…" />
            <Button type="submit" size="sm" variant="primary" disabled={!secret.trim()} pending={token.isPending}>Save token</Button>
          </SettingsRow>
          {tokenError}
        </form>
      </Disclosure>
    </Section>
  );
}

function Runs({ v }: { v: HostSettingsView }) {
  const s = v.settings;
  const save = useSaveSettings();
  const website = useInstance().data?.website;
  const port = (text: string) => {
    const n = Number(text);
    if (!/^\d+$/.test(text) || n > 65535) throw new Error('a port is a number up to 65535');
    return n;
  };
  return (
    <Section
      label="Runs"
      description={
        <>
          Commands PDT runs from a ticket's branch, such as your app's dev server or its tests. Each project defines its
          runs in <code>.pdt/project.toml</code> or in the project's settings.{' '}
          {website && <Link href={`${website}/docs/automation/run-configuration`} external>How runs are set up</Link>}
        </>
      }
    >
      <SettingsRow label="Ports" description="Each run that serves something gets a port from this range, for its link, and for its Tailscale link when remote access is on.">
        <AutoSaveText aria-label="First run port" inputMode="numeric" className="w-20" value={String(s.runPorts.start)}
          onSave={async (t) => save.saveText({ runPorts: { start: port(t), end: s.runPorts.end } })} />
        <span className="text-muted-foreground">–</span>
        <AutoSaveText aria-label="Last run port" inputMode="numeric" className="w-20" value={String(s.runPorts.end)}
          onSave={async (t) => save.saveText({ runPorts: { start: s.runPorts.start, end: port(t) } })} />
      </SettingsRow>
      <SettingsRow label="Max concurrent runs" description="How many runs may be going at once across all projects (tests don't count). Starting one more offers to stop the oldest.">
        <AutoSaveText aria-label="Max concurrent runs" type="number" min={1} max={32} className="w-20" value={String(s.maxConcurrentRuns)}
          onSave={async (t) => save.saveText({ maxConcurrentRuns: Number(t) })} />
      </SettingsRow>
    </Section>
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
        <Select
          aria-label="Default agent"
          className="w-40"
          // A switched-off agent can't be the default (PDT-270).
          options={[...agentChoices, ...v.acpAgents.map((a) => ({ value: a.agent, label: a.label }))].filter((o) => !agentsOff(v).includes(o.value))}
          value={v.settings.defaultAgent}
          onChange={(a) => void agent.save({ defaultAgent: a })}
        />
      </SettingsRow>
      <SettingsRow label="Default view" description="Whether a new agent session is a conversation or the agent's own terminal. You can switch a session later.">
        <SaveNote s={view} />
        <Select aria-label="Default view" className="w-40" options={viewChoices} value={v.settings.defaultSessionMode} onChange={(m) => void view.save({ defaultSessionMode: m })} />
      </SettingsRow>
      <SettingsRow label="Ticket permissions" description="The permission mode a ticket's Start picks first. In an agent's terminal only Bypass changes anything.">
        <SaveNote s={ticketMode} />
        <PermissionModeSelect
          label="Ticket permissions"
          size="md"
          className="w-48"
          value={v.settings.ticketPermissionMode}
          onChange={(m) => void ticketMode.save({ ticketPermissionMode: m })}
        />
      </SettingsRow>
      <SettingsRow label="Session permissions" description="The permission mode a new session picks first (the new tab chooser).">
        <SaveNote s={sessionMode} />
        <PermissionModeSelect
          label="Session permissions"
          size="md"
          className="w-48"
          value={v.settings.sessionPermissionMode}
          onChange={(m) => void sessionMode.save({ sessionPermissionMode: m })}
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
          onChange={(n) => void turns.save({ maxAgentTurns: Number(n) })}
        />
      </SettingsRow>
      <TriageCaps v={v} />
      <SettingsRow label="Credentials" description="Subscription removes ANTHROPIC_API_KEY and OPENAI_API_KEY from agents' environment; pass through keeps them (API keys, Bedrock/Vertex, proxies). A project can override it.">
        <SaveNote s={save} />
        <Select aria-label="Host agent credentials" className="w-72 max-w-full" options={credentialPolicies} value={v.settings.agentCredentials} onChange={(c) => void save.save({ agentCredentials: c })} />
      </SettingsRow>
    </Section>
  );
}

/** One agent's default model: a pick from the models its conversations offer, or typed while it has listed none. */
function DefaultModel({ agent, label, value }: { agent: AgentKind; label: string; value: string | null }) {
  const save = useSaveSettings();
  const caps = useAgentCapabilities(agent);
  const models = caps.data?.models ?? [];
  const set = (model: string | null) => ({ agentModels: { [agent]: model ?? '' } });
  return (
    <SettingsRow label={label}>
      {models.length ? (
        <>
          <SaveNote s={save} />
          <Select
            aria-label={`${label} default model`}
            className="w-48"
            options={modelChoices(models, value, "Agent's default")}
            value={value ?? DEFAULT_MODEL}
            onChange={(m) => void save.save(set(m === DEFAULT_MODEL ? null : m))}
          />
        </>
      ) : (
        <AutoSaveText
          aria-label={`${label} default model`}
          mono
          className="w-48"
          spellCheck={false}
          placeholder="Agent's default"
          value={value ?? ''}
          onSave={async (m) => save.saveText(set(m || null))}
        />
      )}
    </SettingsRow>
  );
}

/**
 * The model each agent starts with when neither the session nor its persona picks one (PDT-265). The list is the one
 * the conversation's model picker shows; until the agent has run a conversation it has none, and the name is typed.
 */
function DefaultModels({ v }: { v: HostSettingsView }) {
  const agents = useAgents();
  return (
    <Section label="Default models" description="The model each agent starts with: conversations, terminals (Claude and Codex), workflow steps and the PM. A model picked in the conversation or set on the persona wins.">
      {agents.all.map((a) => (
        <DefaultModel key={a} agent={a} label={agents.label(a)} value={v.settings.agentModels?.[a] ?? null} />
      ))}
    </Section>
  );
}

const PROGRAM_NAMES: Record<AgentKind, string> = { claude: 'Claude Code', codex: 'Codex', devin: 'Devin' };
const PROGRAM_FIELD: Record<AgentKind, 'claudeProgram' | 'codexProgram' | 'devinProgram'> = {
  claude: 'claudeProgram',
  codex: 'codexProgram',
  devin: 'devinProgram',
};

/**
 * Where each agent's program is (PDT-231): found on the PATH unless a path is set here. Its environment variable wins
 * over the setting, which then shows read-only. Each agent has its switch (PDT-270): a switched-off one isn't missing.
 */
function Programs({ v }: { v: HostSettingsView }) {
  const save = useSaveSettings();
  const qc = useQueryClient();
  return (
    <Section label="Programs" description="Where each agent's program is, and whether PDT uses it. Leave a path empty to use the one on the PATH; new sessions use a change.">
      {v.programs.map((p) => {
        const name = PROGRAM_NAMES[p.agent];
        const setting = v.settings[PROGRAM_FIELD[p.agent]] ?? '';
        const off = offNote(v, p.agent);
        const description = off ?? (p.fromEnv ? `Set by ${p.envVar}, which wins over this setting.`
          : p.found ? (setting ? `Found at ${p.found}.` : `Found on the PATH: ${p.found}. Set a path to use another one.`)
            : setting ? 'Not found at this path.' : `Not found on the PATH. Install it or give its path.`);
        return (
          <SettingsRow key={p.agent} label={name} description={<>{description}{switchBlocked(v, p.agent) && <span className="block">{switchBlocked(v, p.agent)}</span>}</>}>
            <AgentSwitch v={v} agent={p.agent} label={name} />
            {off ? <Badge>Off</Badge> : p.fromEnv ? <EnvBadge name={p.envVar} /> : p.found ? <Badge tone="running">Found</Badge> : <Badge tone={p.agent === 'devin' ? 'neutral' : 'failed'}>Missing</Badge>}
            {p.fromEnv ? (
              <Input aria-label={`${name} program`} mono className="w-64 max-w-full" value={p.fromEnv} disabled />
            ) : (
              <AutoSaveText
                aria-label={`${name} program`}
                mono
                className="w-64 max-w-full"
                spellCheck={false}
                placeholder={p.found ?? `/path/to/${p.agent}`}
                value={setting}
                onSave={async (path) => {
                  await save.saveText({ [PROGRAM_FIELD[p.agent]]: path });
                  // The agents offered (and their logins) follow the new program.
                  void qc.invalidateQueries({ queryKey: hostKeys.checks });
                }}
              />
            )}
          </SettingsRow>
        );
      })}
    </Section>
  );
}

/** Whether the page is open on this machine, so a new port can be reached at the same host name. */
const isLocal = (hostname: string) => ['localhost', '127.0.0.1', '[::1]', '::1'].includes(hostname);

/** Waits (up to a minute) until something answers at `origin`, then opens the same page there. */
async function reopenAt(origin: string) {
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    try {
      await fetch(`${origin}/api/health`, { mode: 'no-cors', cache: 'no-store' });
      window.location.assign(`${origin}${window.location.pathname}`);
      return;
    } catch {
      // Not up yet.
    }
  }
}

/**
 * Where PDT listens (PDT-231): its port and the extra host names its UI may be opened from. Kept on this host and
 * applied when PDT restarts: a banner offers Restart now (through pdt-host). PDT_PORT and PDT_ALLOWED_HOSTS win.
 */
function Network({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const n = v.network;
  const save = (next: NetworkSettings) => api.saveNetwork(next).then((view) => setHostSettings(qc, view));
  const [restarting, setRestarting] = useState(false);
  const [restart, asking] = useAction(async () => {
    const port = n.portFromEnv ?? n.settings.port ?? n.defaultPort;
    await api.restartHost();
    setRestarting(true);
    if (port !== n.listenPort && isLocal(window.location.hostname)) {
      await reopenAt(`${window.location.protocol}//${window.location.hostname}:${port}`);
    } else {
      // The same address: the page reconnects by itself once PDT answers again.
      window.setTimeout(() => void qc.invalidateQueries({ queryKey: hostKeys.settings }), 5000);
    }
  });
  const portText = n.settings.port === null ? '' : String(n.settings.port);
  return (
    <Section
      label="Network"
      description="Where PDT listens. Changes apply after a restart."
      notice={n.restartNeeded && (
        <Alert
          tone="warning"
          action={n.canRestart && <Button size="sm" variant="primary" pending={asking || restarting} onClick={() => void restart()}>Restart now</Button>}
        >
          {restarting ? 'Restarting PDT…'
            : n.canRestart ? 'Restart PDT to use the new settings. Its sessions are interrupted and resume when it is back.'
              : 'Restart PDT to use the new settings: this one was not started by its host supervisor, so stop it and start it again.'}
        </Alert>
      )}
    >
      <SettingsRow
        label="Port"
        description={n.portFromEnv !== null ? 'Set by PDT_PORT, which wins over this setting.'
          : `PDT's web UI and API (default ${n.defaultPort}; now ${n.listenPort}). Leave it empty for the default.`}
      >
        {n.portFromEnv !== null ? (
          <>
            <EnvBadge name="PDT_PORT" />
            <Input aria-label="Port" className="w-24" value={String(n.portFromEnv)} disabled />
          </>
        ) : (
          <AutoSaveText
            aria-label="Port"
            inputMode="numeric"
            className="w-24"
            placeholder={String(n.defaultPort)}
            value={portText}
            onSave={async (t) => {
              if (t && (!/^\d+$/.test(t) || Number(t) > 65535)) throw new Error('a port is a number up to 65535');
              await save({ ...n.settings, port: t ? Number(t) : null });
            }}
          />
        )}
      </SettingsRow>
      <SettingsRow
        label="Allowed hosts"
        description={n.allowedHostsFromEnv !== null ? 'Set by PDT_ALLOWED_HOSTS, which wins over this setting.'
          : 'Other host names the UI may be opened from, with :port when needed; comma-separated. localhost and the tailnet name remote access uses are always allowed.'}
      >
        {n.allowedHostsFromEnv !== null ? (
          <>
            <EnvBadge name="PDT_ALLOWED_HOSTS" />
            <Input aria-label="Allowed hosts" className="w-72 max-w-full" value={n.allowedHostsFromEnv.join(', ')} disabled />
          </>
        ) : (
          <AutoSaveText
            aria-label="Allowed hosts"
            mono
            className="w-72 max-w-full"
            placeholder="pdt.local:7420"
            value={n.settings.allowedHosts.join(', ')}
            onSave={(t) => save({ ...n.settings, allowedHosts: t.split(',').map((h) => h.trim()).filter(Boolean) })}
          />
        )}
      </SettingsRow>
    </Section>
  );
}

const themeChoices: { value: ThemePref; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
];

/** The theme (PDT-231): the same choice as the switch in the rail, kept in this browser. */
function Appearance() {
  const [theme, setTheme] = useThemePref();
  const [state, track] = useSaveState();
  return (
    <Section label="Appearance" description="How PDT looks in this browser. The same choice as the switch in the side rail.">
      <SettingsRow label="Theme" description="System follows your computer's light or dark mode.">
        <SaveStatus state={state} />
        <Select<ThemePref> aria-label="Theme" className="w-40" options={themeChoices} value={theme}
          onChange={(t) => void track(Promise.resolve(setTheme(t)))} />
      </SettingsRow>
    </Section>
  );
}

const TRIAGE_CAPS = [1, 2, 3, 5, 10, 20, 50, 100];

/** The hourly caps on triage runs started by outside input (PDT-202). */
function TriageCaps({ v }: { v: HostSettingsView }) {
  const author = useSaveSettings();
  const repo = useSaveSettings();
  const options = (n: number) => {
    const all = [...new Set([...TRIAGE_CAPS, n])].filter((x) => x > 0).sort((a, b) => a - b);
    return [{ value: '0', label: 'No cap' }, ...all.map((x) => ({ value: String(x), label: `${x} an hour` }))];
  };
  return (
    <>
      <SettingsRow
        label="Triage per author"
        description="The most new issues one author gets triaged an hour (a workflow with an agent step started by a new issue or a webhook). More are skipped, and the Inbox sums them up."
      >
        <SaveNote s={author} />
        <Select<string>
          aria-label="Triage per author"
          className="w-40"
          options={options(v.settings.triagePerAuthorHour)}
          value={String(v.settings.triagePerAuthorHour)}
          onChange={(n) => void author.save({ triagePerAuthorHour: Number(n) })}
        />
      </SettingsRow>
      <SettingsRow label="Triage per repository" description="The same for each repository (or webhook).">
        <SaveNote s={repo} />
        <Select<string>
          aria-label="Triage per repository"
          className="w-40"
          options={options(v.settings.triagePerRepoHour)}
          value={String(v.settings.triagePerRepoHour)}
          onChange={(n) => void repo.save({ triagePerRepoHour: Number(n) })}
        />
      </SettingsRow>
    </>
  );
}

function ProjectsFolder({ v }: { v: HostSettingsView }) {
  const saved = v.settings.projectsDir ?? '';
  const save = useSaveSettings();
  const [dir, setDir] = useState<string | null>(null);
  const commit = (value: string) => {
    if (value.trim() === saved) return setDir(null);
    void save.save({ projectsDir: value }).then((ok) => { if (ok) setDir(null); });
  };
  return (
    <Section label="Projects folder" description="Where “clone by URL” puts new repositories.">
      <SettingsRow stacked label="Folder" description={<>Saved when you leave the field or pick a folder. <SaveNote s={save} /></>}>
        <FolderField
          aria-label="Projects folder"
          title="Projects folder"
          placeholder={v.projectsDir ?? '~/work'}
          start={v.projectsDir ?? ''}
          value={dir ?? saved}
          onChange={setDir}
          onCommit={commit}
        />
      </SettingsRow>
    </Section>
  );
}

/** "The first provider PDT has" in the default provider's picker. */
const FIRST = 'first';

/** Where new documents go (PDT-314): the default provider and the local documents folder; projects can set their own. */
function Documents({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const providers = useDocumentProviders();
  const provider = useSaveSettings();
  const folder = useSaveSettings();
  const saved = v.settings.documentsDir ?? '';
  const [dir, setDir] = useState<string | null>(null);
  const settle = () => void qc.invalidateQueries({ queryKey: documentKeys.all });
  const commit = (value: string) => {
    if (value.trim() === saved) return setDir(null);
    void folder.save({ documentsDir: value }).then((ok) => {
      if (ok) setDir(null);
      settle();
    });
  };
  const list = providers.data?.providers ?? [];
  const first = list[0]?.label;
  return (
    <Section label="Documents" description="Where new docs and sheets go, unless their project says otherwise.">
      <SettingsRow label="Default provider" description={list.length === 0 ? 'No document provider yet: turn on a documents plugin.' : 'New documents go here.'}>
        <SaveNote s={provider} />
        <Select
          aria-label="Default provider"
          className="w-56 max-w-full"
          disabled={list.length === 0}
          options={[{ value: FIRST, label: first ? `The first (${first})` : 'None' }, ...list.map((p) => ({ value: p.id, label: p.label }))]}
          value={v.settings.documentsProvider ?? FIRST}
          onChange={(id) => void provider.save({ documentsProvider: id === FIRST ? '' : id }).then(settle)}
        />
      </SettingsRow>
      <SettingsRow stacked label="Local documents folder" description={<>Where documents kept on this machine go. Blank: PDT's data folder. <SaveNote s={folder} /></>}>
        <FolderField
          aria-label="Local documents folder"
          title="Local documents folder"
          placeholder={providers.data?.folder ?? ''}
          start={saved || (providers.data?.folder ?? '')}
          value={dir ?? saved}
          onChange={setDir}
          onCommit={commit}
        />
      </SettingsRow>
    </Section>
  );
}

function Checks() {
  const qc = useQueryClient();
  const agents = useAgents();
  const off = useAgentsOff();
  const checks = useHostChecks();
  const [refresh, refreshing] = useAction(async () => qc.setQueryData(hostKeys.checks, await api.refreshChecks()));
  return (
    <Section
      label="Checks"
      description="The programs PDT runs on this host."
      actions={<Button size="sm" pending={refreshing} onClick={() => void refresh()}>Run checks again</Button>}
    >
      {checks.isPending && <Skeleton lines={4} label="Loading checks" />}
      {checks.data && (
        <ul className="flex flex-col gap-1.5">
          {[
            ...(['claude', 'codex', 'devin', 'git', 'gitIdentity', 'tailscale'] as const).map((k) => ({ key: k, name: k === 'gitIdentity' ? 'git identity' : k, c: checks.data[k], optional: k === 'devin' })),
            ...Object.entries(checks.data.acp ?? {}).map(([k, c]) => ({ key: k, name: agents.label(k as AgentKind), c, optional: true })),
          ].map(({ key: k, name, c, optional: canLack }) => {
            // An agent switched off is not checked for (PDT-270).
            const switchedOff = off.includes(k as AgentKind);
            // Devin and ACP agents are optional: a host without them is fine.
            const optional = switchedOff || (canLack && !c.found);
            const [status, text]: [Status, string] = switchedOff ? ['idle', 'switched off'] : c.ok ? ['running', 'OK'] : c.found ? ['waiting', 'needs attention'] : optional ? ['idle', 'not installed (optional)'] : ['failed', 'not found'];
            return (
              <li key={k} className="flex min-w-0 items-center gap-2">
                <StatusDot status={status} label={text} />
                <span className="w-24 shrink-0 truncate font-medium" title={name}>{name}</span>
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

/** Get started (PDT-230): reopens the first-run checklist. */
function Setup() {
  const setup = useQuery({ queryKey: hostKeys.setup, queryFn: api.hostSetup });
  const v = setup.data;
  const where = !v ? undefined
    : v.done === STEPS.length ? 'Every step is done.'
      : `${v.done} of ${STEPS.length} steps done.${v.state.dismissed ? ' Dismissed: it no longer opens on its own.' : ''}`;
  return (
    <Section label="Setup" description="The Get started checklist: agents, GitHub, a first project and the default agent.">
      <SettingsRow label="Get started" description={where}>
        <Button size="sm" onClick={() => navigate('/setup')}>Open Get started</Button>
      </SettingsRow>
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
  const preview = useQuery({ queryKey: hostKeys.bugReportPreview, queryFn: api.bugReportPreview });
  const [open, setOpen] = useState(false);
  const p = preview.data;
  const where = !p ? null
    : p.route === 'github' ? `Reports go to ${p.repo} on GitHub as issues, ${p.via === 'login' ? `filed as ${p.githubLogin ?? 'your GitHub login'}`
      : p.via === 'token' ? 'filed with the update token' : 'sent from your browser on GitHub\'s bug form until you connect GitHub'}.`
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
          onChange={(b) => void save.save({ bugReports: b }).then(() => void qc.invalidateQueries({ queryKey: hostKeys.bugReportPreview }))}
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
      body: 'It is deleted from the OS keychain. Updates go on without it, from the public releases.',
      action: 'Remove token',
      danger: true,
    });
    if (!ok) return;
    await api.deleteToken();
    void qc.invalidateQueries({ queryKey: hostKeys.settings });
    void qc.invalidateQueries({ queryKey: hostKeys.update });
  });
  return (
    <Section label="Danger zone" danger description="These always ask first.">
      {!v.tokenPresent ? (
        <EmptyState size="inline">Nothing to remove: no GitHub token is stored.</EmptyState>
      ) : <SettingsRow label="Remove the GitHub token" description="Deletes it from the OS keychain. Updates go on without it.">
        <Button size="sm" variant="danger" pending={removingToken} onClick={() => void removeToken()}>Remove token…</Button>
      </SettingsRow>}
    </Section>
  );
}

export interface HostSettingsPageProps {
  /** A phone: one screen per section, the list of them while `section` is null. */
  drill?: boolean;
  section?: string | null;
  onSection?: (slug: string) => void;
}

export function HostSettingsPage({ drill = false, section = null, onSection }: HostSettingsPageProps = {}) {
  const qc = useQueryClient();
  const view = useHostSettings();
  const whoami = useQuery({ queryKey: hostKeys.whoami, queryFn: api.whoami });
  const v = view.data;
  if (!v) {
    return (
      <Page title="This host">
        {view.error ? (
          <LoadError what="Host settings" error={view.error} onRetry={() => void qc.invalidateQueries({ queryKey: hostKeys.settings })} />
        ) : (
          <Skeleton label="Loading host settings" className="h-40" />
        )}
      </Page>
    );
  }
  // The device being used when it is on the tailnet; otherwise the login this host's Tailscale is signed in as.
  const me = whoami.data?.caller === 'tailnet' && whoami.data.login ? whoami.data.login : v.remote.tailscaleLogin;
  const nodes: Record<string, ReactNode> = {
    'Setup': <Setup />,
    'Updates': <><Updates v={v} /><InstallUpdates policy={v.settings.updatePolicy} /></>,
    'Agents': <><Agents v={v} /><StuckAgents /><DefaultModels v={v} /><Programs v={v} /><AcpAgents v={v} /></>,
    'PM': <PmCheckpointSettings />,
    'Network': <Network v={v} />,
    'Startup': <StartAtLogin />,
    'Remote access': <RemoteAccess v={v} me={me ?? null} />,
    'Runs': <Runs v={v} />,
    'Storage': <Storage />,
    'Projects folder': <ProjectsFolder v={v} />,
    'Documents': <Documents v={v} />,
    'Appearance': <Appearance />,
    'Bug reports': <BugReports v={v} />,
    'Checks': <Checks />,
    'Danger zone': <DangerZone v={v} />,
  };
  return (
    <SettingsScreens
      title="This host"
      description="Settings for the machine running PDT. Projects have their own pages."
      crumb="This host"
      sections={HOST_SECTIONS.map(({ label }) => ({ label, node: nodes[label] }))}
      drill={drill}
      section={section}
      onSection={onSection}
    />
  );
}
