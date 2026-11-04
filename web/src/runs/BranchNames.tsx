import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Alert, Button, Field, Input, Section, SettingsRow, Skeleton } from '@ui';
import { SaveStatus } from '../common/AutoSave';
import { useDebounced } from '../common/useDebounced';
import { LoadError } from '../common/QueryStatus';
import { api } from '../api/client';
import { projectKeys } from '../projects/queries';
import type { BranchPreview } from '../api/gen/BranchPreview';
import type { PartInfo } from '../api/gen/PartInfo';
import type { Project } from '../api/gen/Project';

const SLASH_NOTE = 'Names with a / work with git, but some tools handle them badly.';

/** `bug=fix, docs=docs` from a types map. */
function typesText(types: Record<string, string>): string {
  return Object.entries(types).map(([label, type]) => `${label}=${type}`).join(', ');
}

/** A types map from `bug=fix, docs=docs` (entries without `=` are skipped). */
function parseTypes(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const entry of text.split(',')) {
    const [label, type] = entry.split('=').map((s) => s.trim());
    if (label && type) out[label] = type;
  }
  return out;
}

/**
 * The template new tickets' branches are named from: the repository's `[branch]` when it has one (shown, not
 * editable), else the setting kept on this host. Edits preview live; Save keeps them.
 */
export function BranchNames({ project }: { project: Project }) {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: projectKeys.branchNames(project.id), queryFn: () => api.branchNames(project.id) });
  const storedTemplate = view.data?.setting.template ?? '';
  const storedTypes = typesText(view.data?.setting.types ?? {});
  const [template, setTemplate] = useState('');
  const [types, setTypes] = useState('');
  useEffect(() => setTemplate(storedTemplate), [storedTemplate]);
  useEffect(() => setTypes(storedTypes), [storedTypes]);
  const dirty = template !== storedTemplate || types !== storedTypes;
  const draftTemplate = useDebounced(template, 300);
  const draftTypes = useDebounced(types, 300);
  const preview = useQuery({
    queryKey: projectKeys.branchNamesPreview(project.id, draftTemplate, draftTypes),
    queryFn: () => api.previewBranchNames(project.id, { template: draftTemplate, types: parseTypes(draftTypes) }),
    enabled: dirty && view.data?.source !== 'file',
    placeholderData: keepPreviousData,
  });
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveBranchNames(project.id, { template: template.trim() || null, types: parseTypes(types) }),
    onSuccess: (v) => qc.setQueryData(projectKeys.branchNames(project.id), v),
  });
  const v = view.data;
  const fromFile = v?.source === 'file';
  const shown = dirty && !fromFile ? preview.data : v && { error: null, slash: v.slash, previews: v.previews };
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (dirty) save.mutate(); }}>
      <Section
        label="Branch names"
        description="How PDT names a ticket's branch when work starts. The Start dialog shows the name and lets you change it; a name already in use gets -2, -3 and so on."
        dirty={dirty && !fromFile}
        footer={
          fromFile ? undefined : (
            <>
              {save.isSuccess && !dirty && <SaveStatus state={{ kind: 'saved' }} />}
              <Button type="submit" variant="primary" size="sm" aria-label="Save branch names" disabled={!dirty} pending={save.isPending}>Save</Button>
            </>
          )
        }
      >
        {view.isPending ? (
          <Skeleton lines={3} label="Loading branch names" />
        ) : view.isError ? (
          <LoadError what="Branch names" error={view.error} />
        ) : fromFile ? (
          <SettingsRow label="Template" description="Set by .pdt/project.toml, which wins over this setting.">
            <span className="font-mono text-xs">{v?.template}</span>
          </SettingsRow>
        ) : (
          <>
            <Field label="Template" hint={<PartsHint parts={v?.parts ?? []} />}>
              <Input
                mono
                aria-label="Branch name template"
                value={template}
                placeholder={v?.template ?? 'pdt-{key}-{slug}'}
                maxLength={200}
                spellCheck={false}
                onChange={(e) => { setTemplate(e.target.value); save.reset(); }}
              />
            </Field>
            <Field label="Types" hint="What {type} is for a ticket's labels, as label=type. bug=fix is built in; any other ticket is feat.">
              <Input
                mono
                aria-label="Branch types"
                value={types}
                placeholder="docs=docs, chore=chore"
                spellCheck={false}
                onChange={(e) => { setTypes(e.target.value); save.reset(); }}
              />
            </Field>
          </>
        )}
        {v?.fileError && <Alert tone="error">{v.fileError} Branches get the default names meanwhile.</Alert>}
        {shown?.error && <Alert tone="error">{shown.error}</Alert>}
        {shown?.slash && <Alert tone="warning">{SLASH_NOTE}</Alert>}
        {shown && !shown.error && <Previews previews={shown.previews} />}
        {save.isError && <Alert tone="error">{save.error.message}</Alert>}
      </Section>
    </form>
  );
}

function PartsHint({ parts }: { parts: PartInfo[] }) {
  return (
    <>
      Parts:{' '}
      {parts.map((p, n) => (
        <span key={p.name}>
          {n > 0 && ', '}
          <code title={p.description}>{`{${p.name}}`}</code>
        </span>
      ))}
      . {'{jira|key}'} takes the first that has a value; an empty part is dropped with its separator. Leave it empty for pdt-{'{key}'}-{'{slug}'}.
    </>
  );
}

function Previews({ previews }: { previews: BranchPreview[] }) {
  return (
    <SettingsRow stacked label="Preview">
      <ul aria-label="Branch name previews" className="flex flex-col gap-1 text-xs">
        {previews.map((p) => (
          <li key={p.label} className="flex flex-wrap gap-x-2">
            <span className="text-muted-foreground">{p.label}</span>
            <span className="font-mono break-all">{p.name}</span>
          </li>
        ))}
      </ul>
    </SettingsRow>
  );
}
