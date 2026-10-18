import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Alert, Button, Field, Section, Select, Skeleton, Textarea } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { Project } from '../api/gen/Project';
import { CodeViewer } from '../files/LazyCodeViewer';

/** The Select's value for "no override: use the host setting" (its empty value would show the placeholder). */
const HOST = 'host';
type CredsChoice = CredentialPolicy | typeof HOST;

/**
 * Per-project host settings, as the project page's "Runs on this host" `Section`: the repo's .pdt/project.toml, host
 * overrides of it, and the agent credential policy.
 */
export function ProjectSettingsPanel({ project, className }: { project: Project; className?: string }) {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ['project-settings', project.id], queryFn: () => api.projectSettings(project.id) });
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const [overrides, setOverrides] = useState('');
  const [creds, setCreds] = useState<CredsChoice>(HOST);
  const save = useMutation({
    mutationFn: () =>
      api.saveProjectSettings(project.id, {
        agentCredentials: creds === HOST ? null : creds,
        runOverrides: overrides.trim() ? overrides : null,
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
  }, [settings.data]);
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
      </div>
      {settings.isError && <Alert tone="error">{settings.error.message}</Alert>}
      <Field label="Host overrides (TOML)" hint="Merged over the repository's file, on this host only.">
        <Textarea
          mono
          rows={10}
          spellCheck={false}
          value={overrides}
          onChange={(e) => edit(setOverrides)(e.target.value)}
          className="text-xs"
        />
      </Field>
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
      {save.isError && <Alert tone="error">{save.error.message}</Alert>}
    </Section>
  );
}
