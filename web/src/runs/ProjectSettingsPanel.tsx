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
import type { AutoStartOrder } from '../api/gen/AutoStartOrder';
import type { MergeBy } from '../api/gen/MergeBy';
import { PERMISSION_LABEL, PERMISSION_MODES } from '../conversation/ConversationComposer';
import type { Project } from '../api/gen/Project';
import type { ProjectSettingsInput } from '../api/gen/ProjectSettingsInput';
import type { ProjectSettingsView } from '../api/gen/ProjectSettingsView';
import { CodeViewer } from '../files/LazyCodeViewer';
import { ConnectionPicker } from '../connections/ConnectionPicker';
import { fieldShown, fieldText } from '../connections/kinds';
import type { ProjectSettingsSchema } from '../api/gen/ProjectSettingsSchema';
import type { SettingsSection } from '../common/SettingsScreens';

/** The Select's value for "no override: use the host setting" (its empty value would show the placeholder). */
const HOST = 'host';
type CredsChoice = CredentialPolicy | typeof HOST;

/** PDT's built-in prompts (served by pdtd, so they work without a copy of PDT's repository). */
type SetupGuide = 'set-up-project' | 'set-up-workflow';
/** The agents a set-up prompt can start (in the host's default view). */
const SETUP_AGENTS = ['Claude', 'Codex', 'Devin'] as const;
type SetupAgent = (typeof SETUP_AGENTS)[number];

/**
 * Saves the project's settings: the fields given, and the stored value of every other one. Refuses until the settings
 * have loaded: the PUT always replaces `agentCredentials` and `runOverrides`, so saving without them would clear them.
 */
function useSaveProjectSettings(project: Project, current: ProjectSettingsView | undefined) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (p: Partial<ProjectSettingsInput>) => {
      if (!current) throw new Error("The project's settings have not loaded, so nothing was saved. Try again once they have.");
      return api.saveProjectSettings(project.id, {
        agentCredentials: current.agentCredentials,
        runOverrides: current.runOverrides,
        ...p,
      });
    },
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
  // The integrations a project links to; they change only with PDT.
  const schemas = useQuery({ queryKey: ['project-settings-schemas'], queryFn: api.projectSettingsSchemas, staleTime: Infinity }).data ?? [];
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
    !personal && { label: 'Plugins', node: <Integrations project={project} settings={settings.data} schemas={schemas} /> },
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
            disabled={!settings}
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
            disabled={!settings}
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
const AUTO_START_ORDER: { value: AutoStartOrder; label: string }[] = [
  { value: 'priority', label: 'Priority, then rank' },
  { value: 'rank', label: 'Backlog rank' },
];
const START_DELAYS: { value: string; label: string }[] = [
  { value: '0', label: 'None: at once' },
  { value: '30', label: '30 seconds' },
  { value: '60', label: '1 minute' },
  { value: '120', label: '2 minutes' },
  { value: '300', label: '5 minutes' },
  { value: '600', label: '10 minutes' },
  { value: '1800', label: '30 minutes' },
];
const NO_CAP = 'none';
const CAPS = [1, 2, 3, 4, 5, 6, 8, 10];

/**
 * The project's policies (Phase 7c): who may merge, whether an automated merge needs a person's approval or green CI
 * (PDT-151), the cap on
 * running tickets, auto-start and its quiet period, and whether the PM asks before acting (PDT-81). Merge and Start enforce them, whatever
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
          <SettingsRow
            label="An automated merge waits for green CI"
            description="A workflow's or an agent's merge of a pull request waits for CI on its head to pass, and goes back to the coder when it fails, whatever GitHub's branch protection requires. A repository without CI merges at once."
          >
            <Switch
              aria-label="An automated merge waits for green CI"
              checked={p?.requireGreenCi ?? true}
              disabled={!settings || save.isPending}
              onChange={(on) => save.mutate({ requireGreenCi: on }, done)}
            />
          </SettingsRow>
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
        description="Starts tickets on its own, skipping blocked and conflicting ones, while the project is under its cap."
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
      {p && p.autoStart !== 'off' && (
        <SettingsRow
          label="Auto-start picks by"
          description="Priority: the most urgent ticket first (urgent, high, medium, low, none), then backlog rank. Backlog rank: the top ticket first, whatever its priority. To do tickets always come before Ready ones."
        >
          <Select<AutoStartOrder>
            aria-label="Auto-start picks by"
            className="w-56 max-w-full"
            options={AUTO_START_ORDER}
            value={p.autoStartOrder}
            disabled={!settings}
            onChange={(o) => save.mutate({ autoStartOrder: o }, done)}
          />
        </SettingsRow>
      )}
      {p && p.autoStart !== 'off' && (
        <SettingsRow
          label="Quiet period before auto-start"
          description="Auto-start takes a ticket only once it has gone this long unchanged since it was created, edited, linked or commented on, so blockers linked just after it was made count. Start is never delayed."
        >
          <Select<string>
            aria-label="Quiet period before auto-start"
            className="w-56 max-w-full"
            options={START_DELAYS.some((d) => d.value === String(p.startDelaySeconds))
              ? START_DELAYS
              : [...START_DELAYS, { value: String(p.startDelaySeconds), label: `${p.startDelaySeconds} seconds` }]}
            value={String(p.startDelaySeconds)}
            disabled={!settings}
            onChange={(v) => save.mutate({ startDelaySeconds: Number(v) }, done)}
          />
        </SettingsRow>
      )}
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

/** A form's text for each integration edited, by its id: `connection` and each field's key. */
type IntegrationForms = Record<string, Record<string, string>>;

/** An integration's stored settings as the form's text (a field with a default shows it until set). */
function integrationText(schema: ProjectSettingsSchema, stored: Record<string, unknown> | undefined): Record<string, string> {
  const text: Record<string, string> = { connection: typeof stored?.connection === 'string' ? stored.connection : '' };
  for (const f of schema.fields) text[f.key] = stored && f.key in stored ? fieldText(f, stored[f.key]) : f.default ?? '';
  return text;
}

/** The settings the form's text makes: every field sent, so an emptied one is cleared (or takes its default). */
function integrationInput(schema: ProjectSettingsSchema, text: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = { connection: text.connection ?? '' };
  for (const f of schema.fields) {
    const t = text[f.key] ?? '';
    out[f.key] = f.type === 'lines' ? t.split('\n').map((l) => l.trim()).filter(Boolean) : t.trim();
  }
  return out;
}

/**
 * The project's integrations (PDT-174): each integration that a project links to declares its connection kind and
 * fields, and its part of the form is made from them (Jira's issues and Done, Buildkite's pipelines). Kept on this
 * host, never in the repository. Typed values stay until saved (the panel re-renders on every poll).
 */
function Integrations({ project, settings, schemas }: { project: Project; settings: ProjectSettingsView | undefined; schemas: ProjectSettingsSchema[] }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const [forms, setForms] = useState<IntegrationForms>({});
  const stored = (id: string) => settings?.integrations?.[id];
  const textOf = (s: ProjectSettingsSchema) => forms[s.integration] ?? integrationText(s, stored(s.integration));
  const set = (s: ProjectSettingsSchema, key: string, value: string) => setForms((f) => ({ ...f, [s.integration]: { ...textOf(s), [key]: value } }));
  const dirty = Object.keys(forms).length > 0;
  const done = { onSuccess: () => { setForms({}); setSaved(true); } };
  const submit = () => {
    if (!dirty || save.isPending) return;
    const integrations = Object.fromEntries(schemas.filter((s) => forms[s.integration]).map((s) => [s.integration, integrationInput(s, forms[s.integration]!)]));
    save.mutate({ integrations }, done);
  };
  return (
    <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
      <Section
        label="Plugins"
        dirty={dirty}
        description="Link this project to the services its issues and CI live in. Kept on this host, never written to the repository."
        footer={
          <>
            {save.isError && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
            <SaveFooter label="Save integrations" dirty={dirty} saved={saved} pending={save.isPending} />
          </>
        }
      >
        {schemas.map((s) => {
          const text = textOf(s);
          return (
            <Fragment key={s.integration}>
              <SettingsRow label={s.label} description={s.description}>
                <div className="flex w-80 max-w-full flex-col gap-1">
                  <ConnectionPicker kind={s.connectionKind} label={`${s.label} connection`} value={text.connection ?? ''} disabled={!settings} onChange={(c) => set(s, 'connection', c)} />
                  {stored(s.integration) && (
                    <Button size="sm" variant="secondary" onClick={() => save.mutate({ integrations: { [s.integration]: null } }, done)}>Unlink {s.label}</Button>
                  )}
                </div>
              </SettingsRow>
              {text.connection && s.fields.filter((f) => fieldShown(f, text)).map((f) => (
                <SettingsRow key={f.key} label={f.label} description={f.hint}>
                  {f.type === 'lines'
                    ? <Textarea mono aria-label={f.label} rows={3} className="w-80 max-w-full" placeholder={f.placeholder} value={text[f.key] ?? ''} onChange={(e) => set(s, f.key, e.target.value)} />
                    : f.type === 'choice'
                      ? <Select<string> aria-label={f.label} className="w-56 max-w-full" options={f.options ?? []} value={text[f.key] ?? ''} onChange={(v) => set(s, f.key, v)} />
                      : <Input aria-label={f.label} className="w-80 max-w-full" placeholder={f.placeholder} value={text[f.key] ?? ''} onChange={(e) => set(s, f.key, e.target.value)} />}
                </SettingsRow>
              ))}
            </Fragment>
          );
        })}
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
