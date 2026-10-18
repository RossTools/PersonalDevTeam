import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Alert, Button, Field, Section, Select, Skeleton, Textarea } from '@ui';
import { cn } from 'cn';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { Project } from '../api/gen/Project';
import { CodeViewer } from '../files/CodeViewer';

/** The Select's value for "no override: use the host setting" (its empty value would show the placeholder). */
const HOST = 'host';
type CredsChoice = CredentialPolicy | typeof HOST;

/**
 * Per-project host settings: overrides of the repo's .pdt/project.toml, and the agent credential policy. Rendered as
 * `Section`s; `className` replaces the default padding when the parent is a `Page`.
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
    <div className={cn('flex flex-col gap-6 text-sm', className ?? 'p-4')}>
      <h2 className="text-base font-semibold">{project.name}: settings on this host</h2>
      <Section
        label="Repository configuration"
        description={<span className="font-mono">{config.data?.filePath ?? '.pdt/project.toml'}</span>}
      >
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
      </Section>
      <Section
        label="On this host"
        actions={
          <>
            <span role="status" className="text-xs text-muted-foreground">{save.isSuccess ? 'Saved' : ''}</span>
            <Button variant="primary" size="sm" pending={save.isPending} onClick={() => save.mutate()}>Save</Button>
          </>
        }
      >
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
    </div>
  );
}
