import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Alert, Button, Field, MenuButton, Section, Select, SettingsRow, Skeleton, Switch, Textarea } from '@ui';
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
  return (
    <>
      {settings.isError && <Alert tone="error">Project settings could not be loaded: {settings.error.message}</Alert>}
      {!personal && <RunConfiguration project={project} settings={settings.data} setUpEntries={agentEntries('set-up-project')} />}
      <Instructions project={project} settings={settings.data} writeEntries={agentEntries('set-up-workflow')} />
      <Agents project={project} settings={settings.data} />
      {!personal && <Merging project={project} settings={settings.data} />}
      <Pipeline project={project} settings={settings.data} personal={personal} />
    </>
  );
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
 * running tickets and auto-start. Merge and Start enforce them, whatever a workflow says. Each saves at once.
 */
function Pipeline({ project, settings, personal }: { project: Project; settings: ProjectSettingsView | undefined; personal: boolean }) {
  const save = useSaveProjectSettings(project, settings);
  const [saved, setSaved] = useSavedNote();
  const p = settings?.policy;
  const done = { onSuccess: () => setSaved(true) };
  const cap = p?.maxRunningAttempts ? String(p.maxRunningAttempts) : NO_CAP;
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
        </>
      )}
      <SettingsRow label="Running tickets at once" description="Start asks before going over it; auto-start never does (one when there is no cap).">
        <Select<string>
          aria-label="Running tickets at once"
          className="w-40 max-w-full"
          options={[{ value: NO_CAP, label: 'No cap' }, ...CAPS.map((n) => ({ value: String(n), label: String(n) }))]}
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
    </Section>
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
