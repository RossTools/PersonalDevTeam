import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Alert, Button, Field, MenuButton, Section, Select, Skeleton, Switch, Textarea } from '@ui';
import { api } from '../api/client';
import { runAction } from '../common/useAction';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { agentOf, newSession } from '../sessions/newSession';
import { useOpenSession } from '../tickets/actions';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { Project } from '../api/gen/Project';
import { CodeViewer } from '../files/LazyCodeViewer';

/** The Select's value for "no override: use the host setting" (its empty value would show the placeholder). */
const HOST = 'host';
type CredsChoice = CredentialPolicy | typeof HOST;

/** PDT's built-in prompts (served by pdtd, so they work without a copy of PDT's repository). */
type SetupGuide = 'set-up-project' | 'set-up-workflow';
/** The conversations a set-up prompt can start. */
const SETUP_AGENTS = ['Claude', 'Codex', 'Devin'] as const;
type SetupAgent = (typeof SETUP_AGENTS)[number];

/**
 * Per-project host settings, as the project page's "Runs on this host" `Section`: the repo's .pdt/project.toml, host
 * overrides of it, the agent instructions PDT gives every agent session here, and the agent credential policy.
 * Nothing here is written to the repository.
 */
export function ProjectSettingsPanel({ project, className }: { project: Project; className?: string }) {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ['project-settings', project.id], queryFn: () => api.projectSettings(project.id) });
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const ignored = ignoredNote(config.data?.effective?.ignored);
  const [overrides, setOverrides] = useState('');
  const [creds, setCreds] = useState<CredsChoice>(HOST);
  const [instructions, setInstructions] = useState('');
  const [mergeViaPr, setMergeViaPr] = useState(false);
  const openSession = useOpenSession();
  const agents = useAgentAvailability();
  const save = useMutation({
    mutationFn: () =>
      api.saveProjectSettings(project.id, {
        agentCredentials: creds === HOST ? null : creds,
        runOverrides: overrides.trim() ? overrides : null,
        agentInstructions: instructions,
        mergeViaPr,
      }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['project-settings', project.id] });
      void qc.invalidateQueries({ queryKey: ['run-config', project.id] });
    },
  });
  const { reset } = save;
  useEffect(() => {
    if (!settings.data) return;
    setOverrides(settings.data.runOverrides ?? '');
    setCreds(settings.data.agentCredentials ?? HOST);
    setInstructions(settings.data.agentInstructions ?? '');
    setMergeViaPr(settings.data.mergeViaPr);
  }, [settings.data]);
  // An agent session in the main checkout whose first message is one of PDT's set-up prompts.
  const setUp = (guide: SetupGuide, agent: SetupAgent) =>
    void runAction(async () => {
      const initialPrompt = await api.guide(guide);
      const s = await api.createSession({ ...newSession(project.id, null, agent), initialPrompt });
      void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
      openSession(s);
    });
  // The agents this host has (all until the checks load), a logged-out one saying so.
  const agentEntries = (guide: SetupGuide) => () =>
    SETUP_AGENTS.filter((a) => agents.offered(agentOf(a)!)).map((a) => {
      const hint = agents.hint(agentOf(a)!);
      return { label: hint ? `${a} · ${hint}` : a, onSelect: () => setUp(guide, a) };
    });
  // "Saved" describes what is on screen: an edit clears it.
  const edit = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    reset();
  };

  return (
    <Section
      label="Runs on this host"
      className={className}
      actions={
        <>
          <span role="status" className="text-xs text-muted-foreground">{save.isSuccess ? 'Saved' : ''}</span>
          <MenuButton label="Set up with an agent" placement="bottom end" entries={agentEntries('set-up-project')}>
            <Button size="sm">Set up with an agent</Button>
          </MenuButton>
          <Button variant="primary" size="sm" pending={save.isPending} onClick={() => save.mutate()}>Save</Button>
        </>
      }
    >
      <div className="flex flex-col gap-1.5">
        <p className="flex flex-wrap items-baseline gap-x-2 text-xs">
          <span className="font-medium text-foreground">Repository configuration</span>
          <span className="font-mono text-muted-foreground">{config.data?.filePath ?? '.pdt/project.toml'}</span>
        </p>
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
      </div>
      {settings.isError && <Alert tone="error">{settings.error.message}</Alert>}
      <Field
        label="Host overrides (TOML)"
        hint="Merged over the repository's file, on this host only. Can hold the whole configuration, so nothing needs committing."
      >
        <Textarea
          mono
          rows={10}
          spellCheck={false}
          value={overrides}
          onChange={(e) => edit(setOverrides)(e.target.value)}
          className="text-xs"
        />
      </Field>
      <Field
        label="Agent instructions"
        hint="Given to every Claude, Codex and Devin session in this project, with PDT's own context, on top of the repository's CLAUDE.md or AGENTS.md. Kept on this host, never written to the repository. New sessions get changes."
      >
        <Textarea
          rows={6}
          spellCheck={false}
          placeholder={'How agents should work here, e.g.:\n- Make a branch and worktree through PDT for every change.\n- Run `make check` before pushing.'}
          value={instructions}
          onChange={(e) => edit(setInstructions)(e.target.value)}
          className="text-xs"
        />
      </Field>
      <MenuButton label="Write the instructions with an agent" placement="bottom start" entries={agentEntries('set-up-workflow')}>
        <Button size="sm" className="self-start">Write instructions with an agent</Button>
      </MenuButton>
      <Field label="Agent credentials" inline>
        <Select<CredsChoice>
          options={[
            { value: HOST, label: `Use the host setting (${settings.data?.effectiveCredentials ?? '…'})` },
            { value: 'subscription', label: 'Subscription (remove API keys)' },
            { value: 'pass_through', label: 'Pass through' },
          ]}
          value={creds}
          onChange={edit(setCreds)}
        />
      </Field>
      <Field
        label="Merging"
        hint="On: Merge… only turns on auto-merge for the ticket's pull request (or asks the agent to open one); it never merges into the base branch on this machine."
      >
        <Switch checked={mergeViaPr} onChange={edit(setMergeViaPr)}>Merge through pull requests</Switch>
      </Field>
      {save.isError && <Alert tone="error">{save.error.message}</Alert>}
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
