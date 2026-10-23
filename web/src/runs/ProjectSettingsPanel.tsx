import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { Alert, Button, Field, Input, MenuButton, Section, Select, SettingsRow, Skeleton, Switch, Textarea } from '@ui';
import { api } from '../api/client';
import { runAction } from '../common/useAction';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { agentOf, byDefault, choiceFor, newSession } from '../sessions/newSession';
import { useOpenSession } from '../tickets/actions';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { AutoStart } from '../api/gen/AutoStart';
import type { MergeBy } from '../api/gen/MergeBy';
import { PERMISSION_LABEL, PERMISSION_MODES } from '../conversation/ConversationComposer';
import type { Project } from '../api/gen/Project';
import type { ProjectSettingsInput } from '../api/gen/ProjectSettingsInput';
import type { ProjectSettingsView } from '../api/gen/ProjectSettingsView';
import { CodeViewer } from '../files/LazyCodeViewer';
import { ConnectionPicker } from '../connections/ConnectionPicker';
import type { SettingsSection } from '../common/SettingsScreens';

/** The Select's value for "no override: use the host setting" (its empty value would show the placeholder). */
const HOST = 'host';
type CredsChoice = CredentialPolicy | typeof HOST;

/** PDT's built-in prompts (served by pdtd, so they work without a copy of PDT's repository). */
type SetupGuide = 'set-up-project' | 'set-up-workflow';
/** The agents a set-up prompt can start (in the host's default view). */
const SETUP_AGENTS = ['Claude', 'Codex', 'Devin'] as const;
type SetupAgent = (typeof SETUP_AGENTS)[number];

/** Saves the project's settings: the fields given, and the stored value of every other one. */
function useSaveProjectSettings(project: Project, current: ProjectSettingsView | undefined) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (p: Partial<ProjectSettingsInput>) =>
      api.saveProjectSettings(project.id, {
        agentCredentials: current?.agentCredentials ?? null,
        runOverrides: current?.runOverrides ?? null,
        ...p,
      }),
    onSuccess: (v) => {
      qc.setQueryData(['project-settings', project.id], v);
      void qc.invalidateQueries({ queryKey: ['run-config', project.id] });
    },
  });
}

/**
 * Per-project host settings, as sections of the project's page: the repo's .pdt/project.toml and host overrides of
 * it, the agent instructions PDT gives every agent session here, and the agent credential policy. Each section saves
 * only its own fields. Nothing here is written to the repository. A Personal project has no run configuration or
 * merging.
 */
export function ProjectSettingsPanel({ project }: { project: Project }) {
  const { notice, sections } = useProjectSettingsSections(project);
  return (
    <>
      {notice}
      {sections.map((s) => <Fragment key={s.label}>{s.node}</Fragment>)}
    </>
  );
}

/** The panel's sections one by one (a phone shows each on a screen of its own), and a load error to show above them. */
export function useProjectSettingsSections(project: Project): { notice: ReactNode; sections: SettingsSection[] } {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ['project-settings', project.id], queryFn: () => api.projectSettings(project.id) });
  const openSession = useOpenSession();
  const agents = useAgentAvailability();
  const defaults = useAgentDefaults();
  // A Personal project has no repository: no runs, tests or merging (spec 7b §7).
  const personal = project.kind === 'personal';
  // An agent session in the main checkout, in the host's default view, whose first message is one of PDT's set-up
  // prompts.
  const setUp = (guide: SetupGuide, agent: SetupAgent) =>
    void runAction(async () => {
      const initialPrompt = await api.guide(guide);
      const choice = choiceFor(agentOf(agent)!, defaults.mode);
      const s = await api.createSession({ ...newSession(project.id, null, choice), initialPrompt });
      void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
      openSession(s);
    });
  // The agents this host has (all until the checks load), the default first, a logged-out one saying so.
  const agentEntries = (guide: SetupGuide) => () =>
    byDefault(SETUP_AGENTS.filter((a) => agents.offered(agentOf(a)!)), (a) => a, defaults).map((a) => {
      const hint = agents.hint(agentOf(a)!);
      return { label: hint ? `${a} · ${hint}` : a, onSelect: () => setUp(guide, a) };
    });
  const sections: (SettingsSection | false)[] = [
    !personal && { label: 'Runs on this host', node: <RunConfiguration project={project} settings={settings.data} setUpEntries={agentEntries('set-up-project')} /> },
    { label: 'Agent instructions', node: <Instructions project={project} settings={settings.data} writeEntries={agentEntries('set-up-workflow')} /> },
    { label: 'Agents', node: <Agents project={project} settings={settings.data} /> },
    !personal && { label: 'Review', node: <Merging project={project} settings={settings.data} /> },
    { label: 'Pipeline and policies', node: <Pipeline project={project} settings={settings.data} personal={personal} /> },
    !personal && { label: 'Jira and Buildkite', node: <Integrations project={project} settings={settings.data} /> },
  ];
  return {
    notice: settings.isError ? <Alert tone="error">Project settings could not be loaded: {settings.error.message}</Alert> : null,
    sections: sections.filter((x): x is SettingsSection => x !== false),
  };
}

type Entries = () => { label: string; onSelect: () => void }[];

/** A form's Save with its "Saved" note, for a section footer. */
function SaveFooter({ label, dirty, saved, pending }: { label: string; dirty: boolean; saved: boolean; pending: boolean }) {
  return (
    <>
      {saved && !dirty && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      <Button type="submit" variant="primary" size="sm" aria-label={label} disabled={!dirty} pending={pending}>Save</Button>
    </>
  );
}

function RunConfiguration({ project, settings, setUpEntries }: { project: Project; settings: ProjectSettingsView | undefined; setUpEntries: Entries }) {
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const ignored = ignoredNote(config.data?.effective?.ignored);
  const stored = settings?.runOverrides ?? '';
  const [overrides, setOverrides] = useState('');
  useEffect(() => setOverrides(stored), [stored]);
  const save = useSaveProjectSettings(project, settings);
  const dirty = overrides !== stored;
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (dirty) save.mutate({ runOverrides: overrides.trim() ? overrides : null }); }}>
      <Section
        label="Runs on this host"
        description="What Run starts for this project's branches: the repository's file, plus overrides kept on this host."
        actions={
          <MenuButton label="Set up with an agent" placement="bottom end" entries={setUpEntries}>
            <Button size="sm">Set up with an agent</Button>
          </MenuButton>
        }
        dirty={dirty}
        footer={<SaveFooter label="Save run overrides" dirty={dirty} saved={save.isSuccess} pending={save.isPending} />}
      >
        <SettingsRow stacked label="Repository configuration" description={<span className="font-mono">{config.data?.filePath ?? '.pdt/project.toml'}</span>}>
          {config.isPending ? (
            <Skeleton lines={4} label="Loading the run configuration" />
          ) : config.isError ? (
            <Alert tone="error">{config.error.message}</Alert>
          ) : config.data.file ? (
            <div className="max-h-80 overflow-auto rounded-md border border-border">
              <CodeViewer name={config.data.filePath ?? 'project.toml'} text={config.data.file} />
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">This repository has no run configuration file.</p>
          )}
          {config.data?.error && <Alert tone="error">{config.data.error}</Alert>}
          {ignored && <Alert>{ignored}</Alert>}
        </SettingsRow>
        <Field
          label="Host overrides (TOML)"
          hint="Merged over the repository's file, on this host only. Can hold the whole configuration, so nothing needs committing."
        >
          <Textarea
            mono
            rows={10}
            spellCheck={false}
            value={overrides}
            onChange={(e) => { setOverrides(e.target.value); save.reset(); }}
            className="text-xs"
          />
        </Field>
        {save.isError && <Alert tone="error">{save.error.message}</Alert>}
      </Section>
    </form>
  );
}

function Instructions({ project, settings, writeEntries }: { project: Project; settings: ProjectSettingsView | undefined; writeEntries: Entries }) {
  const stored = settings?.agentInstructions ?? '';
  const [text, setText] = useState('');
  useEffect(() => setText(stored), [stored]);
  const save = useSaveProjectSettings(project, settings);
  const dirty = text !== stored;
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (dirty) save.mutate({ agentInstructions: text }); }}>
      <Section
        label="Agent instructions"
        description="Given to every Claude, Codex and Devin session in this project, with PDT's own context, on top of the repository's CLAUDE.md or AGENTS.md. Kept on this host, never written to the repository. New sessions get changes."
        actions={
          <MenuButton label="Write the instructions with an agent" placement="bottom end" entries={writeEntries}>
            <Button size="sm">Write instructions with an agent</Button>
          </MenuButton>
        }
        dirty={dirty}
        footer={<SaveFooter label="Save agent instructions" dirty={dirty} saved={save.isSuccess} pending={save.isPending} />}
      >
        <div>
          <Textarea
            aria-label="Agent instructions"
            rows={6}
            spellCheck={false}
            placeholder={'How agents should work here, e.g.:\n- Make a branch and worktree through PDT for every change.\n- Run `make check` before pushing.'}
            value={text}
            onChange={(e) => { setText(e.target.value); save.reset(); }}
            className="text-xs"
          />
        </div>
        {save.isError && <Alert tone="error">{save.error.message}</Alert>}
      </Section>
    </form>
  );
}

/** The agent settings: selects, so each saves at once (spec §6.1). */
function Agents({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  return (
    <Section label="Agents">
      <Credentials project={project} settings={settings} />
      <StepPermissionMode project={project} settings={settings} />
    </Section>
  );
}

/** "Saved" for a couple of seconds after `setSaved(true)`. */
function useSavedNote(): [saved: boolean, setSaved: (on: boolean) => void] {
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const t = setTimeout(() => setSaved(false), 2000);
    return () => clearTimeout(t);
  }, [saved]);
  return [saved, setSaved];
}

function Credentials({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const value: CredsChoice = settings?.agentCredentials ?? HOST;
  return (
    <SettingsRow label="Agent credentials" description="What agents in this project may use to sign in.">
      {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
      {saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      <Select<CredsChoice>
        aria-label="Agent credentials"
        className="w-72 max-w-full"
        options={[
          { value: HOST, label: `Use the host setting (${settings?.effectiveCredentials ?? '…'})` },
          { value: 'subscription', label: 'Subscription (remove API keys)' },
          { value: 'pass_through', label: 'Pass through' },
        ]}
        value={value}
        disabled={!settings}
        onChange={(c) => save.mutate({ agentCredentials: c === HOST ? null : c }, { onSuccess: () => setSaved(true) })}
      />
    </SettingsRow>
  );
}

/** The Select's value for "no override" (sent as "", which clears it). */
const NO_MODE = 'none';
type StepModeChoice = PermissionMode | typeof NO_MODE;

/** The permission mode of every persona's session (workflow agent steps) here, over the persona's own (spec 7b §3). */
function StepPermissionMode({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const value: StepModeChoice = settings?.agentPermissionMode ?? NO_MODE;
  return (
    <SettingsRow
      label="Agent permission mode for workflow steps"
      description="Used by every persona's session in this project, over the persona's own mode. None: the persona's, else the host default."
    >
      {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
      {saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      <Select<StepModeChoice>
        aria-label="Agent permission mode for workflow steps"
        className="w-56 max-w-full"
        options={[{ value: NO_MODE, label: "None (the persona's)" }, ...PERMISSION_MODES.map((m) => ({ value: m, label: PERMISSION_LABEL[m] }))]}
        value={value}
        disabled={!settings}
        onChange={(m) => save.mutate({ agentPermissionMode: m === NO_MODE ? '' : m }, { onSuccess: () => setSaved(true) })}
      />
    </SettingsRow>
  );
}

/** Merge through pull requests (review spec §2.7): a switch, so it saves at once. */
function Merging({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  return (
    <Section label="Review">
      <SettingsRow
        label="Merge through pull requests"
        description="Merge… only turns on auto-merge for the ticket's pull request (or asks the agent to open one); it never merges into the base branch on this machine."
      >
        {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
        <Switch
          aria-label="Merge through pull requests"
          checked={settings?.mergeViaPr ?? false}
          disabled={!settings || save.isPending}
          onChange={(on) => save.mutate({ mergeViaPr: on })}
        />
      </SettingsRow>
    </Section>
  );
}

const MERGE_BY: { value: MergeBy; label: string }[] = [
  { value: 'agents', label: 'Agents, workflows and people' },
  { value: 'user', label: 'Only a person' },
  { value: 'external', label: 'Nobody in PDT (merged elsewhere)' },
];
const AUTO_START: { value: AutoStart; label: string }[] = [
  { value: 'off', label: 'Off' },
  { value: 'todo', label: 'To do tickets' },
  { value: 'ready', label: 'To do, then Ready tickets' },
];
const NO_CAP = 'none';
const CAPS = [1, 2, 3, 4, 5, 6, 8, 10];

/**
 * The project's policies (Phase 7c): who may merge, whether an automated merge needs a person's approval, the cap on
 * running tickets, auto-start, and whether the PM asks before acting (PDT-81). Merge and Start enforce them, whatever
 * a workflow says. Each saves at once.
 */
function Pipeline({ project, settings, personal }: { project: Project; settings: ProjectSettingsView | undefined; personal: boolean }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const p = settings?.policy;
  const done = { onSuccess: () => setSaved(true) };
  const cap = p?.maxRunningAttempts ? String(p.maxRunningAttempts) : NO_CAP;
  // Typed text until it is saved (the panel re-renders on every poll, so it is never overwritten while typed).
  const storedExempt = (p?.approvalExemptLabels ?? []).join(', ');
  const [exempt, setExempt] = useState<string | null>(null);
  const saveExempt = () => {
    if (save.isPending) return;
    if (exempt === null || exempt === storedExempt) return setExempt(null);
    const labels = exempt.split(',').map((l) => l.trim()).filter(Boolean);
    save.mutate({ approvalExemptLabels: labels }, { onSuccess: () => { setExempt(null); setSaved(true); } });
  };
  return (
    <Section
      label="Pipeline and policies"
      description="What workflows and agents may do here. Merge and Start keep to these whatever a workflow asks."
      footer={
        <>
          {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
          {saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
        </>
      }
    >
      {!personal && (
        <>
          <SettingsRow label="Who may merge" description="Merge… in PDT is a person's merge; a workflow's or an agent's is automated.">
            <Select<MergeBy>
              aria-label="Who may merge"
              className="w-72 max-w-full"
              options={MERGE_BY}
              value={p?.mergeBy ?? 'agents'}
              disabled={!settings}
              onChange={(m) => save.mutate({ mergeBy: m }, done)}
            />
          </SettingsRow>
          <SettingsRow
            label="A person approves before an automated merge"
            description="Approve in Changes, or Accept in the Inbox, at the branch's current head."
          >
            <Switch
              aria-label="A person approves before an automated merge"
              checked={p?.requireApproval ?? false}
              disabled={!settings || save.isPending}
              onChange={(on) => save.mutate({ requireApproval: on }, done)}
            />
          </SettingsRow>
          {p?.requireApproval && (
            <SettingsRow
              label="Labels that merge without approval"
              description="A ticket with any of these labels may merge automatically without it. Comma-separated."
            >
              <Input
                aria-label="Labels that merge without approval"
                className="w-72 max-w-full"
                placeholder="e.g. automerge"
                value={exempt ?? storedExempt}
                disabled={!settings}
                onChange={(e) => setExempt(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') saveExempt(); }}
                onBlur={saveExempt}
              />
            </SettingsRow>
          )}
        </>
      )}
      <SettingsRow
        label="Running tickets at once"
        description="Counts every ticket with an active attempt, however it started. Start asks before going over it; auto-start never goes over it, and starts one at a time when there is no cap."
      >
        <Select<string>
          aria-label="Running tickets at once"
          className="w-56 max-w-full"
          options={[{ value: NO_CAP, label: p?.autoStart && p.autoStart !== 'off' ? 'No cap (auto-start: 1)' : 'No cap' }, ...CAPS.map((n) => ({ value: String(n), label: String(n) }))]}
          value={cap}
          disabled={!settings}
          onChange={(v) => save.mutate({ maxRunningAttempts: v === NO_CAP ? 0 : Number(v) }, done)}
        />
      </SettingsRow>
      <SettingsRow
        label="Auto-start"
        description="Starts tickets in rank order, skipping blocked and conflicting ones, while the project is under its cap."
      >
        <Select<AutoStart>
          aria-label="Auto-start"
          className="w-56 max-w-full"
          options={AUTO_START}
          value={p?.autoStart ?? 'off'}
          disabled={!settings}
          onChange={(a) => save.mutate({ autoStart: a }, done)}
        />
      </SettingsRow>
      <SettingsRow
        label="The PM acts without asking"
        description="The project manager creates and edits this project's tickets and messages its sessions without asking you first. Off: you confirm each change."
      >
        <Switch
          aria-label="The PM acts without asking"
          checked={p?.pmActs ?? false}
          disabled={!settings || save.isPending}
          onChange={(on) => save.mutate({ pmActs: on }, done)}
        />
      </SettingsRow>
    </Section>
  );
}

interface JiraForm { connection: string; scope: string; done: string }
interface BuildkiteForm { connection: string; pipelines: string }

/**
 * The project's Jira project and Buildkite pipelines (Phase 7e): issues assigned to you become tickets, Done moves the
 * issue, and the branches' builds show as CI. Kept on this host, never in the repository. Typed values stay until
 * saved (the panel re-renders on every poll).
 */
function Integrations({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const j = settings?.jira;
  const b = settings?.buildkite;
  const [jira, setJira] = useState<JiraForm | null>(null);
  const [bk, setBk] = useState<BuildkiteForm | null>(null);
  const jv: JiraForm = jira ?? { connection: j?.connection ?? '', scope: j?.scope ?? '', done: j?.done ?? 'Done' };
  const bv: BuildkiteForm = bk ?? { connection: b?.connection ?? '', pipelines: (b?.pipelines ?? []).join(', ') };
  const dirty = jira !== null || bk !== null;
  const done = { onSuccess: () => { setJira(null); setBk(null); setSaved(true); } };
  const submit = () => {
    if (!dirty || save.isPending) return;
    save.mutate({
      ...(jira ? { jira: { connection: jv.connection, scope: jv.scope, done: jv.done } } : {}),
      ...(bk ? { buildkite: { connection: bv.connection, pipelines: bv.pipelines.split(',').map((p) => p.trim()).filter(Boolean) } } : {}),
    }, done);
  };
  return (
    <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
      <Section
        label="Jira and Buildkite"
        dirty={dirty}
        description="For a project whose issues are in Jira and whose CI is Buildkite. Kept on this host, never written to the repository."
        footer={
          <>
            {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
            <SaveFooter label="Save Jira and Buildkite" dirty={dirty} saved={saved} pending={save.isPending} />
          </>
        }
      >
        <SettingsRow
          label="Jira"
          description="Issues assigned to you in this scope become Draft tickets (checked every minute), and moving a linked ticket to Done moves its issue."
        >
          <div className="flex w-80 max-w-full flex-col gap-1">
            <ConnectionPicker kind="jira" label="Jira connection" value={jv.connection} disabled={!settings} onChange={(c) => setJira({ ...jv, connection: c })} />
            {j && <Button size="sm" variant="secondary" onClick={() => save.mutate({ jira: { connection: '' } }, done)}>Unlink Jira</Button>}
          </div>
        </SettingsRow>
        {jv.connection && (
          <>
            <SettingsRow label="Jira issues" description="JQL for this project's issues. Empty: every issue the account can see.">
              <Input aria-label="Jira issues" className="w-80 max-w-full" placeholder="project = ABC" value={jv.scope} onChange={(e) => setJira({ ...jv, scope: e.target.value })} />
            </SettingsRow>
            <SettingsRow label="Done in Jira" description="The transition (or status) a ticket moved to Done moves its issue to.">
              <Input aria-label="Done in Jira" className="w-56 max-w-full" value={jv.done} onChange={(e) => setJira({ ...jv, done: e.target.value })} />
            </SettingsRow>
          </>
        )}
        <SettingsRow label="Buildkite" description="Builds of the tickets' branches show as their CI, and failures are told to the agent like GitHub's.">
          <div className="flex w-80 max-w-full flex-col gap-1">
            <ConnectionPicker kind="buildkite" label="Buildkite connection" value={bv.connection} disabled={!settings} onChange={(c) => setBk({ ...bv, connection: c })} />
            {b && <Button size="sm" variant="secondary" onClick={() => save.mutate({ buildkite: { connection: '' } }, done)}>Unlink Buildkite</Button>}
          </div>
        </SettingsRow>
        {bv.connection && (
          <SettingsRow label="Pipelines" description="The pipelines (slugs) that build this project's branches. Comma-separated.">
            <Input aria-label="Pipelines" className="w-80 max-w-full" placeholder="my-app, my-app-e2e" value={bv.pipelines} onChange={(e) => setBk({ ...bv, pipelines: e.target.value })} />
          </SettingsRow>
        )}
      </Section>
    </form>
  );
}

/** Sections written for a newer version are skipped, not refused; name them so a missing feature is not a mystery. */
function ignoredNote(ignored: string[] | undefined): string | null {
  if (!ignored?.length) return null;
  const names = ignored.map((s) => `[${s}]`).join(', ');
  return ignored.length === 1
    ? `Skipped ${names}: this version does not know that section.`
    : `Skipped ${names}: this version does not know those sections.`;
}
