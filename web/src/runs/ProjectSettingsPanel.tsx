import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { Project } from '../api/gen/Project';

/** Per-project host settings: overrides of the repo's .pdt/project.toml, and the agent credential policy. */
export function ProjectSettingsPanel({ project }: { project: Project }) {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ['project-settings', project.id], queryFn: () => api.projectSettings(project.id) });
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const [overrides, setOverrides] = useState('');
  const [creds, setCreds] = useState<CredentialPolicy | ''>('');
  useEffect(() => {
    if (!settings.data) return;
    setOverrides(settings.data.runOverrides ?? '');
    setCreds(settings.data.agentCredentials ?? '');
  }, [settings.data]);
  const save = useMutation({
    mutationFn: () =>
      api.saveProjectSettings(project.id, { agentCredentials: creds || null, runOverrides: overrides.trim() ? overrides : null }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['project-settings', project.id] });
      void qc.invalidateQueries({ queryKey: ['run-config', project.id] });
    },
  });

  return (
    <div className="flex flex-col gap-3 p-4 text-sm overflow-auto">
      <h2 className="text-base font-semibold">{project.name}: settings on this host</h2>
      <section aria-label="Repository configuration" className="flex flex-col gap-1">
        <h3 className="text-xs uppercase tracking-wider text-muted-foreground">{config.data?.filePath ?? '.pdt/project.toml'}</h3>
        <pre className="rounded-pdt bg-surface border border-border p-2 text-xs font-mono whitespace-pre-wrap">
          {config.data?.file ?? 'This repository has no run configuration file.'}
        </pre>
        {config.data?.error && <p className="text-xs text-failed">{config.data.error}</p>}
      </section>
      <label className="flex flex-col gap-1">
        <span className="text-xs uppercase tracking-wider text-muted-foreground">Host overrides (TOML)</span>
        <textarea
          aria-label="Host overrides (TOML)"
          value={overrides}
          onChange={(e) => setOverrides(e.target.value)}
          rows={10}
          spellCheck={false}
          className="rounded-pdt bg-surface border border-border p-2 font-mono text-xs text-fg"
        />
      </label>
      <label className="flex items-center gap-2">
        <span>Agent credentials</span>
        <select
          aria-label="Agent credentials"
          value={creds}
          onChange={(e) => setCreds(e.target.value as CredentialPolicy | '')}
          className="rounded-pdt bg-surface border border-border px-1 py-0.5"
        >
          <option value="">Use the host setting ({settings.data?.effectiveCredentials ?? '…'})</option>
          <option value="subscription">Subscription (remove API keys)</option>
          <option value="pass_through">Pass through</option>
        </select>
      </label>
      <div className="flex items-center gap-2">
        <Button variant="primary" disabled={save.isPending} onClick={() => save.mutate()}>Save</Button>
        <span role="status" className="text-xs text-muted-foreground">{save.isSuccess ? 'Saved' : ''}</span>
      </div>
      {save.isError && <p role="alert" className="text-xs text-failed">{save.error.message}</p>}
    </div>
  );
}
