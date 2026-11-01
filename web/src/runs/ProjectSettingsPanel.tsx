import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { Alert, Button, Field, MenuButton, Section, Select, SettingsRow, Skeleton, Switch, Textarea } from '@ui';
import { AutoSaveRow, AutoSaveText, SaveStatus } from '../common/AutoSave';
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
import type { ResolveConflicts } from '../api/gen/ResolveConflicts';
import type { TriageApproval } from '../api/gen/TriageApproval';
import type { MergeBy } from '../api/gen/MergeBy';
import { PERMISSION_LABEL, PERMISSION_MODES } from '../conversation/ConversationComposer';
import type { Project } from '../api/gen/Project';
import type { PushDef } from '../api/gen/PushDef';
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
        // Raw TOML keeps an explicit Save (PDT-231): half-typed TOML is never saved.
        footer={
          <>
            {save.isSuccess && !dirty && <SaveStatus state={{ kind: 'saved' }} />}
            <Button type="submit" variant="primary" size="sm" aria-label="Save run overrides" disabled={!dirty} pending={save.isPending}>Save</Button>
          </>
        }
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
        {config.data?.effective?.forge === 'local' && <LocalForgeRow push={config.data.effective.push ?? null} />}
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
  const save = useSaveProjectSettings(project, settings);
  return (
    <Section
      label="Agent instructions"
      description="Given to every Claude, Codex and Devin session in this project, with PDT's own context, on top of the repository's CLAUDE.md or AGENTS.md. Kept on this host, never written to the repository. New sessions get changes."
      actions={
        <MenuButton label="Write the instructions with an agent" placement="bottom end" entries={writeEntries}>
          <Button size="sm">Write instructions with an agent</Button>
        </MenuButton>
      }
    >
      <SettingsRow stacked label="Instructions" description="Saved when you leave the field (or press Ctrl+Enter).">
        <AutoSaveText
          multiline
          aria-label="Agent instructions"
          rows={6}
          spellCheck={false}
          placeholder={'How agents should work here, e.g.:\n- Make a branch and worktree through PDT for every change.\n- Run `make check` before pushing.'}
          value={settings?.agentInstructions ?? ''}
          disabled={!settings}
          onSave={(text) => save.mutateAsync({ agentInstructions: text })}
          className="text-xs"
        />
      </SettingsRow>
    </Section>
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

function Credentials({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  const value: CredsChoice = settings?.agentCredentials ?? HOST;
  return (
    <AutoSaveRow label="Agent credentials" description="What agents in this project may use to sign in.">
      {(track) => (
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
          onChange={(c) => void track(save.mutateAsync({ agentCredentials: c === HOST ? null : c }))}
        />
      )}
    </AutoSaveRow>
  );
}

/** The Select's value for "no override" (sent as "", which clears it). */
const NO_MODE = 'none';
type StepModeChoice = PermissionMode | typeof NO_MODE;

/** The permission mode of every persona's session (workflow agent steps) here, over the persona's own (spec 7b §3). */
function StepPermissionMode({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  const value: StepModeChoice = settings?.agentPermissionMode ?? NO_MODE;
  return (
    <AutoSaveRow
      label="Agent permission mode for workflow steps"
      description="Used by every persona's session in this project, over the persona's own mode. None: the persona's, else the host default."
    >
      {(track) => (
        <Select<StepModeChoice>
          aria-label="Agent permission mode for workflow steps"
          className="w-56 max-w-full"
          options={[{ value: NO_MODE, label: "None (the persona's)" }, ...PERMISSION_MODES.map((m) => ({ value: m, label: PERMISSION_LABEL[m] }))]}
          value={value}
          disabled={!settings}
          onChange={(m) => void track(save.mutateAsync({ agentPermissionMode: m === NO_MODE ? '' : m }))}
        />
      )}
    </AutoSaveRow>
  );
}

/** Merge through pull requests (review spec §2.7): a switch, so it saves at once. */
function Merging({ project, settings }: { project: Project; settings: ProjectSettingsView | undefined }) {
  const save = useSaveProjectSettings(project, settings);
  return (
    <Section label="Review">
      <AutoSaveRow
        label="Merge through pull requests"
        description="Merge… only turns on auto-merge for the ticket's pull request (or asks the agent to open one); it never merges into the base branch on this machine."
      >
        {(track) => (
          <Switch
            aria-label="Merge through pull requests"
            checked={settings?.mergeViaPr ?? false}
            disabled={!settings || save.isPending}
            onChange={(on) => void track(save.mutateAsync({ mergeViaPr: on }))}
          />
        )}
      </AutoSaveRow>
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
const TRIAGE_APPROVAL: { value: TriageApproval; label: string }[] = [
  { value: 'inbox', label: 'Wait in the Inbox' },
  { value: 'off', label: 'Off: stay in the backlog' },
];
const RESOLVE_CONFLICTS: { value: ResolveConflicts; label: string }[] = [
  { value: 'merge_time', label: 'At merge time' },
  { value: 'immediately', label: 'Immediately' },
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
  const set = (p: Partial<ProjectSettingsInput>) => save.mutateAsync(p);
  const p = settings?.policy;
  const cap = p?.maxRunningAttempts ? String(p.maxRunningAttempts) : NO_CAP;
  return (
    <Section
      label="Pipeline and policies"
      description="What workflows and agents may do here. Merge and Start keep to these whatever a workflow asks."
    >
      {!personal && (
        <>
          <AutoSaveRow label="Who may merge" description="Merge… in PDT is a person's merge; a workflow's or an agent's is automated.">
            {(track) => (
              <Select<MergeBy>
                aria-label="Who may merge"
                className="w-72 max-w-full"
                options={MERGE_BY}
                value={p?.mergeBy ?? 'agents'}
                disabled={!settings}
                onChange={(m) => void track(set({ mergeBy: m }))}
              />
            )}
          </AutoSaveRow>
          <AutoSaveRow
            label="A person approves before an automated merge"
            description="Approve in Changes, or Accept in the Inbox, at the branch's current head."
          >
            {(track) => (
              <Switch
                aria-label="A person approves before an automated merge"
                checked={p?.requireApproval ?? false}
                disabled={!settings || save.isPending}
                onChange={(on) => void track(set({ requireApproval: on }))}
              />
            )}
          </AutoSaveRow>
          {p?.requireApproval && (
            <SettingsRow
              label="Labels that merge without approval"
              description="A ticket with any of these labels may merge automatically without it. Comma-separated."
            >
              <AutoSaveText
                aria-label="Labels that merge without approval"
                className="w-72 max-w-full"
                placeholder="e.g. automerge"
                value={(p.approvalExemptLabels ?? []).join(', ')}
                disabled={!settings}
                onSave={(text) => set({ approvalExemptLabels: text.split(',').map((l) => l.trim()).filter(Boolean) })}
              />
            </SettingsRow>
          )}
          <AutoSaveRow
            label="An automated merge waits for green CI"
            description="A workflow's or an agent's merge of a pull request waits for CI on its head to pass, and goes back to the coder when it fails, whatever GitHub's branch protection requires. A repository without CI merges at once."
          >
            {(track) => (
              <Switch
                aria-label="An automated merge waits for green CI"
                checked={p?.requireGreenCi ?? true}
                disabled={!settings || save.isPending}
                onChange={(on) => void track(set({ requireGreenCi: on }))}
              />
            )}
          </AutoSaveRow>
          <AutoSaveRow
            label="When to resolve conflicts"
            description="At merge time: a conflict with the base while a ticket is in code, review or QA is shown on the ticket but doesn't interrupt its agent. When the ticket is next to merge, the base is merged in automatically, generated files are regenerated (merge.regenerate in .pdt/project.toml), and only real conflicts go to an agent. Immediately: the agent is told as soon as its pull request conflicts."
          >
            {(track) => (
              <Select<ResolveConflicts>
                aria-label="When to resolve conflicts"
                className="w-56 max-w-full"
                options={RESOLVE_CONFLICTS}
                value={p?.resolveConflicts ?? 'merge_time'}
                disabled={!settings}
                onChange={(v) => void track(set({ resolveConflicts: v }))}
              />
            )}
          </AutoSaveRow>
        </>
      )}
      <AutoSaveRow
        label="Running tickets at once"
        description="Counts every ticket with an active attempt, however it started. Start asks before going over it; auto-start never goes over it, and starts one at a time when there is no cap."
      >
        {(track) => (
          <Select<string>
            aria-label="Running tickets at once"
            className="w-56 max-w-full"
            options={[{ value: NO_CAP, label: p?.autoStart && p.autoStart !== 'off' ? 'No cap (auto-start: 1)' : 'No cap' }, ...CAPS.map((n) => ({ value: String(n), label: String(n) }))]}
            value={cap}
            disabled={!settings}
            onChange={(v) => void track(set({ maxRunningAttempts: v === NO_CAP ? 0 : Number(v) }))}
          />
        )}
      </AutoSaveRow>
      <AutoSaveRow
        label="Auto-start"
        description="Starts tickets on its own, skipping blocked and conflicting ones, while the project is under its cap. The built-in Pick up tickets workflow does it with these settings; edit or copy it in Workflows to pick up tickets your own way."
      >
        {(track) => (
          <Select<AutoStart>
            aria-label="Auto-start"
            className="w-56 max-w-full"
            options={AUTO_START}
            value={p?.autoStart ?? 'off'}
            disabled={!settings}
            onChange={(a) => void track(set({ autoStart: a }))}
          />
        )}
      </AutoSaveRow>
      {p && p.autoStart !== 'off' && (
        <AutoSaveRow
          label="Auto-start picks by"
          description="Priority: the most urgent ticket first (urgent, high, medium, low, none), then backlog rank. Backlog rank: the top ticket first, whatever its priority. To do tickets always come before Ready ones."
        >
          {(track) => (
            <Select<AutoStartOrder>
              aria-label="Auto-start picks by"
              className="w-56 max-w-full"
              options={AUTO_START_ORDER}
              value={p.autoStartOrder}
              disabled={!settings}
              onChange={(o) => void track(set({ autoStartOrder: o }))}
            />
          )}
        </AutoSaveRow>
      )}
      {p && p.autoStart !== 'off' && (
        <AutoSaveRow
          label="Quiet period before auto-start"
          description="Auto-start takes a ticket only once it has gone this long unchanged since it was created, edited, linked or commented on, so blockers linked just after it was made count. Start is never delayed."
        >
          {(track) => (
            <Select<string>
              aria-label="Quiet period before auto-start"
              className="w-56 max-w-full"
              options={START_DELAYS.some((d) => d.value === String(p.startDelaySeconds))
                ? START_DELAYS
                : [...START_DELAYS, { value: String(p.startDelaySeconds), label: `${p.startDelaySeconds} seconds` }]}
              value={String(p.startDelaySeconds)}
              disabled={!settings}
              onChange={(v) => void track(set({ startDelaySeconds: Number(v) }))}
            />
          )}
        </AutoSaveRow>
      )}
      <AutoSaveRow
        label="The PM acts without asking"
        description="The project manager creates and edits this project's tickets and messages its sessions without asking you first. Off: you confirm each change."
      >
        {(track) => (
          <Switch
            aria-label="The PM acts without asking"
            checked={p?.pmActs ?? false}
            disabled={!settings || save.isPending}
            onChange={(on) => void track(set({ pmActs: on }))}
          />
        )}
      </AutoSaveRow>
      <AutoSaveRow
        label="Triaged tickets"
        description="A ticket triage drafts from an issue waits in your Inbox: Approve moves it to To do (letting in one that came from outside PDT), Reject to Won't do. Off: it stays in the backlog until you move it."
      >
        {(track) => (
          <Select<TriageApproval>
            aria-label="Triaged tickets"
            className="w-56 max-w-full"
            options={TRIAGE_APPROVAL}
            value={p?.triageApproval ?? 'inbox'}
            disabled={!settings}
            onChange={(t) => void track(set({ triageApproval: t }))}
          />
        )}
      </AutoSaveRow>
    </Section>
  );
}

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
 * fields, and its part of the page is made from them (Jira's issues and Done, Buildkite's pipelines). Kept on this
 * host, never in the repository. Each field saves as it changes (PDT-231): a connection or choice at once, text on
 * blur or Enter.
 */
function Integrations({ project, settings, schemas }: { project: Project; settings: ProjectSettingsView | undefined; schemas: ProjectSettingsSchema[] }) {
  const save = useSaveProjectSettings(project, settings);
  const stored = (id: string) => settings?.integrations?.[id];
  /** Saves the integration's settings with one field changed (every field is sent, so the rest stay as they are). */
  const saveField = (s: ProjectSettingsSchema, key: string, value: string) =>
    save.mutateAsync({ integrations: { [s.integration]: integrationInput(s, { ...integrationText(s, stored(s.integration)), [key]: value }) } });
  return (
    <Section
      label="Plugins"
      description="Link this project to the services its issues and CI live in. Kept on this host, never written to the repository."
    >
      {schemas.map((s) => {
        const text = integrationText(s, stored(s.integration));
        return (
          <Fragment key={s.integration}>
            <AutoSaveRow label={s.label} description={s.description}>
              {(track) => (
                <div className="flex w-80 max-w-full flex-col gap-1">
                  <ConnectionPicker kind={s.connectionKind} label={`${s.label} connection`} value={text.connection ?? ''} disabled={!settings}
                    onChange={(c) => void track(saveField(s, 'connection', c))} />
                  {stored(s.integration) && (
                    <Button size="sm" variant="secondary" onClick={() => void track(save.mutateAsync({ integrations: { [s.integration]: null } }))}>Unlink {s.label}</Button>
                  )}
                </div>
              )}
            </AutoSaveRow>
            {text.connection && s.fields.filter((f) => fieldShown(f, text)).map((f) =>
              f.type === 'choice' ? (
                <AutoSaveRow key={f.key} label={f.label} description={f.hint}>
                  {(track) => (
                    <Select<string> aria-label={f.label} className="w-56 max-w-full" options={f.options ?? []} value={text[f.key] ?? ''}
                      onChange={(v) => void track(saveField(s, f.key, v))} />
                  )}
                </AutoSaveRow>
              ) : (
                <SettingsRow key={f.key} label={f.label} description={f.hint}>
                  <AutoSaveText
                    multiline={f.type === 'lines'}
                    mono={f.type === 'lines'}
                    rows={3}
                    aria-label={f.label}
                    className="w-80 max-w-full"
                    placeholder={f.placeholder ?? undefined}
                    value={text[f.key] ?? ''}
                    onSave={(v) => saveField(s, f.key, v)}
                  />
                </SettingsRow>
              ),
            )}
          </Fragment>
        );
      })}
    </Section>
  );
}

/** Sections written for a newer version are skipped, not refused; name them so a missing feature is not a mystery. */
/** Where the local forge's merges go (PDT-245): only the base here, or pushed on to a remote. */
function LocalForgeRow({ push }: { push: PushDef | null }) {
  return push ? (
    <SettingsRow
      label="Local forge"
      description={`PDT merges each ticket into its base once CI passes, then pushes the base to this remote with your own git credentials.${push.branches ? ' Ticket branches are pushed too.' : ''}`}
    >
      <span className="font-mono text-xs text-muted-foreground">{push.remote}</span>
    </SettingsRow>
  ) : (
    <SettingsRow label="Local forge" description="PDT merges each ticket into its base once CI passes.">
      <span className="text-xs text-muted-foreground">Pushes nowhere ([push] remote names a remote)</span>
    </SettingsRow>
  );
}

function ignoredNote(ignored: string[] | undefined): string | null {
  if (!ignored?.length) return null;
  const names = ignored.map((s) => `[${s}]`).join(', ');
  return ignored.length === 1
    ? `Skipped ${names}: this version does not know that section.`
    : `Skipped ${names}: this version does not know those sections.`;
}
