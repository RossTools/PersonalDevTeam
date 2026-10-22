// New workflow (Phase 7d spec §5): a template, its params, a name and where it applies. It starts off, so it can be
// tried before it is enabled.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input, Select, Switch, Textarea } from '@ui';
import { api } from '../api/client';
import { ConnectionPicker } from '../connections/ConnectionPicker';
import type { Project } from '../api/gen/Project';
import type { Scope } from '../api/gen/Scope';
import type { TemplateParam } from '../api/gen/TemplateParam';

/** The value a param starts with in the form. */
function initial(p: TemplateParam): unknown {
  if (p.default !== undefined && p.default !== null) return p.default;
  return p.type === 'boolean' ? false : p.type === 'choice' ? (p.choices ?? [])[0] : p.type === 'number' ? '' : '';
}

export function ParamsForm({ params, values, onChange }: { params: TemplateParam[]; values: Record<string, unknown>; onChange: (v: Record<string, unknown>) => void }) {
  const set = (id: string, v: unknown) => onChange({ ...values, [id]: v });
  return (
    <>
      {params.map((p) => {
        const v = values[p.id];
        const label = <>{p.title}{p.required && <span className="text-muted-foreground"> *</span>}</>;
        return (
          <Field key={p.id} label={label} hint={p.description || undefined}>
            {p.type === 'connection' ? <ConnectionPicker label={p.title} kind={p.connectionKind} value={String(v ?? '')} onChange={(n) => set(p.id, n)} />
              : p.type === 'boolean' ? <Switch checked={v === true} onChange={(on) => set(p.id, on)} />
              : p.type === 'choice' ? <Select options={(p.choices ?? []).map((c) => ({ value: c, label: c }))} value={String(v ?? '')} onChange={(c) => set(p.id, c)} />
                : p.type === 'text' ? <Textarea rows={3} value={String(v ?? '')} onChange={(e) => set(p.id, e.target.value)} />
                  : <Input type={p.type === 'number' ? 'number' : 'text'} value={String(v ?? '')}
                    onChange={(e) => set(p.id, p.type === 'number' ? (e.target.value === '' ? '' : Number(e.target.value)) : e.target.value)} />}
          </Field>
        );
      })}
    </>
  );
}

const startValues = (params: TemplateParam[]) => Object.fromEntries(params.map((p) => [p.id, initial(p)]));

export function NewWorkflowDialog({ projects, templateId, onClose, onCreated }: {
  projects: Project[]; templateId?: string; onClose: () => void; onCreated: (id: string) => void;
}) {
  const qc = useQueryClient();
  const templates = useQuery({ queryKey: ['workflow-templates'], queryFn: () => api.listWorkflowTemplates() });
  const [picked, setPicked] = useState(templateId ?? 'builtin:blank');
  const template = templates.data?.find((t) => t.id === picked);
  const [values, setValues] = useState<Record<string, unknown> | null>(null);
  const [name, setName] = useState<string | null>(null);
  const [where, setWhere] = useState<string>('host');
  const params = template?.params ?? [];
  const vals = values ?? startValues(params);
  const create = useMutation({
    mutationFn: () => {
      const scope: Scope = where === 'host' || where === 'personal' ? where : 'project';
      const filled = Object.fromEntries(Object.entries(vals).filter(([, v]) => v !== ''));
      return api.instantiateWorkflowTemplate(picked, {
        params: filled, name: name?.trim() || null, scope, projectId: scope === 'project' ? where : null,
      });
    },
    onSuccess: (w) => {
      void qc.invalidateQueries({ queryKey: ['workflows'] });
      onCreated(w.id);
    },
  });
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      dismissable={false}
      title="New workflow"
      description="It starts off: dry run it, then turn it on."
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!template} pending={create.isPending} onClick={() => create.mutate()}>Create</Button></>}
    >
      <div className="flex flex-col gap-3">
        <Field label="Template" hint={template?.description || undefined}>
          <Select
            className="w-full"
            placeholder={templates.isLoading ? 'Loading…' : 'Choose a template'}
            options={(templates.data ?? []).map((t) => ({ value: t.id, label: t.builtin ? t.name : `${t.name} (yours)` }))}
            value={picked}
            onChange={(id) => { setPicked(id); setValues(null); setName(null); }}
          />
        </Field>
        <ParamsForm params={params} values={vals} onChange={setValues} />
        <Field label="Name">
          <Input value={name ?? template?.doc.name ?? ''} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Applies to" hint="A project's workflow sees only its events; one for this host sees every project's.">
          <Select
            className="w-full"
            options={[
              { value: 'host', label: 'This host (every project)' },
              ...projects.map((p) => ({ value: p.id, label: p.name })),
              { value: 'personal', label: 'Personal (no project)' },
            ]}
            value={where}
            onChange={setWhere}
          />
        </Field>
        {create.error && <Alert tone="error">{create.error.message}</Alert>}
      </div>
    </Dialog>
  );
}
