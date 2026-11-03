// A template: its params and document. Built-in ones are read-only; the user's own can be edited:
// name, description, params, and the document as text with `{{params.<id>}}`. On a phone it is view-only (PDT-120).
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Plus, Trash2 } from 'lucide-react';
import {
  Alert, Badge, Button, Checkbox, Field, IconButton, Input, MenuButton, SectionLabel, SegmentedControl, Select, Skeleton, Textarea, toast, useConfirm, ViewHeader,
} from '@ui';
import { api } from '../api/client';
import type { ParamType } from '../api/gen/ParamType';
import type { TemplateParam } from '../api/gen/TemplateParam';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import type { WorkflowTemplate } from '../api/gen/WorkflowTemplate';
import { runAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { GraphView } from './GraphView';
import type { Catalog } from './model';
import { TextView } from './TextView';
import { workflowKeys } from './queries';

const TYPES: { value: ParamType; label: string }[] = [
  { value: 'string', label: 'Text' }, { value: 'text', label: 'Long text' }, { value: 'number', label: 'Number' },
  { value: 'boolean', label: 'Yes or no' }, { value: 'choice', label: 'Choice' }, { value: 'connection', label: 'Connection' },
  { value: 'list', label: 'List' }, { value: 'duration', label: 'Duration (ms)' },
];

/** A default typed into the params table, as the param's type wants it. */
function typedDefault(type: ParamType, text: string): unknown {
  if (text === '') return undefined;
  if (type === 'number' || type === 'duration') return Number.isNaN(Number(text)) ? text : Number(text);
  if (type === 'boolean') return text === 'true' ? true : text === 'false' ? false : text;
  if (type === 'list') return text.split(',').map((t) => t.trim()).filter(Boolean);
  return text;
}

function ParamsTable({ params, onChange, readOnly }: { params: TemplateParam[]; onChange: (p: TemplateParam[]) => void; readOnly: boolean }) {
  const set = (i: number, change: Partial<TemplateParam>) => onChange(params.map((p, j) => (j === i ? { ...p, ...change } : p)));
  return (
    <div className="flex flex-col gap-2">
      {params.length === 0 && <p className="text-sm text-muted-foreground">No parameters. Write {'{{params.<id>}}'} in the document and add one here.</p>}
      {params.map((p, i) => (
        <div key={i} className="grid grid-cols-2 gap-2 rounded-md border border-border p-2 md:grid-cols-[8rem_1fr_9rem_1fr_auto_auto]">
          <Input mono aria-label={`Param ${i + 1} id`} placeholder="id" value={p.id} readOnly={readOnly} onChange={(e) => set(i, { id: e.target.value.trim() })} />
          <Input aria-label={`Param ${i + 1} title`} placeholder="Title" value={p.title} readOnly={readOnly} onChange={(e) => set(i, { title: e.target.value })} />
          <Select aria-label={`Param ${i + 1} type`} options={TYPES} value={p.type} disabled={readOnly} onChange={(type) => set(i, { type })} />
          <Input aria-label={`Param ${i + 1} default`} placeholder={p.type === 'choice' ? 'Choices, comma-separated' : p.type === 'list' ? 'Default, comma-separated' : 'Default'} readOnly={readOnly}
            value={p.type === 'choice' ? (p.choices ?? []).join(', ') : p.default === undefined || p.default === null ? '' : Array.isArray(p.default) ? p.default.join(', ') : String(p.default)}
            onChange={(e) => (p.type === 'choice'
              ? set(i, { choices: e.target.value.split(',').map((c) => c.trim()).filter(Boolean) })
              : set(i, { default: typedDefault(p.type, e.target.value) }))} />
          <Checkbox checked={p.required} disabled={readOnly} onChange={(required) => set(i, { required })}>Required</Checkbox>
          {!readOnly && <IconButton label={`Remove param ${i + 1}`} size="sm" onClick={() => onChange(params.filter((_, j) => j !== i))}><Trash2 aria-hidden /></IconButton>}
        </div>
      ))}
      {!readOnly && (
        <Button size="sm" variant="secondary" className="self-start"
          onClick={() => onChange([...params, { id: `param${params.length + 1}`, title: `Param ${params.length + 1}`, description: '', type: 'string', choices: [], required: false }])}>
          <Plus aria-hidden /> Add parameter
        </Button>
      )}
    </div>
  );
}

export function TemplatePage({ id, catalog, onUse, onDeleted }: { id: string; catalog: Catalog; onUse: (templateId: string) => void; onDeleted: () => void }) {
  const t = useQuery({ queryKey: workflowKeys.template(id), queryFn: () => api.getWorkflowTemplate(id) });
  if (t.error) return <Alert tone="error" className="m-3">The template could not be loaded: {t.error.message}</Alert>;
  if (!t.data) return <Skeleton lines={8} className="p-4" />;
  return <Loaded key={`${t.data.id}:${t.data.updatedAt}`} t={t.data} catalog={catalog} onUse={onUse} onDeleted={onDeleted} />;
}

function Loaded({ t, catalog, onUse, onDeleted }: { t: WorkflowTemplate; catalog: Catalog; onUse: (id: string) => void; onDeleted: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const phone = useMediaQuery(PHONE_QUERY);
  const readOnly = t.builtin || phone;
  const [name, setName] = useState(t.name);
  const [description, setDescription] = useState(t.description);
  const [params, setParams] = useState(t.params);
  const [doc, setDoc] = useState<WorkflowDoc>(t.doc);
  const [view, setView] = useState<'graph' | 'text'>('graph');
  const dirty = name !== t.name || description !== t.description || JSON.stringify(params) !== JSON.stringify(t.params) || JSON.stringify(doc) !== JSON.stringify(t.doc);
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveWorkflowTemplate(t.id, { name, description, params, doc }),
    onSuccess: (saved) => {
      qc.setQueryData(workflowKeys.template(t.id), saved);
      void qc.invalidateQueries({ queryKey: workflowKeys.templates });
      toast.success('Template saved');
    },
  });
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={t.name}
        meta={<>{t.builtin ? <Badge tone="neutral">Built in</Badge> : <span>Your template</span>}<span>{t.params.length} parameters</span></>}
        actions={phone ? undefined : (
          <>
            <Button size="sm" variant="primary" onClick={() => onUse(t.id)} disabled={dirty} title={dirty ? 'Save first' : undefined}>Use this template</Button>
            {!t.builtin && (
              <MenuButton label="More template actions" entries={() => [{
                label: 'Delete…', danger: true, onSelect: () => void runAction(async () => {
                  const { ok } = await confirm({ title: `Delete the template ${t.name}?`, body: 'Workflows made from it stay.', action: 'Delete template', danger: true });
                  if (!ok) return;
                  await api.deleteWorkflowTemplate(t.id);
                  void qc.invalidateQueries({ queryKey: workflowKeys.templates });
                  onDeleted();
                }),
              }]} />
            )}
          </>
        )}
      />
      <div className="flex min-h-0 flex-1 flex-col overflow-auto">
        <div className="flex flex-col gap-3 p-3">
          {readOnly && <p className="text-sm text-muted-foreground">{t.description}{t.builtin && " Built-in templates can't be changed: use it, then edit the workflow."}</p>}
          {!readOnly && (
            <>
              <Field label="Name"><Input value={name} onChange={(e) => setName(e.target.value)} /></Field>
              <Field label="Description"><Textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} /></Field>
            </>
          )}
          <SectionLabel>Parameters</SectionLabel>
          <ParamsTable params={params} onChange={setParams} readOnly={readOnly} />
          {save.error && <Alert tone="error">{save.error.message}</Alert>}
          {!readOnly && (
            <div className="flex gap-2">
              <Button variant="primary" disabled={!dirty} pending={save.isPending} onClick={() => save.mutate()}>Save template</Button>
              {dirty && <Button variant="secondary" onClick={() => { setName(t.name); setDescription(t.description); setParams(t.params); setDoc(t.doc); }}>Discard</Button>}
            </div>
          )}
        </div>
        <div className="flex items-center gap-2 border-y border-border px-3 py-1.5">
          <SegmentedControl aria-label="Document view" size="sm" value={view} onChange={setView}
            options={[{ value: 'graph', label: 'Graph' }, { value: 'text', label: 'Text' }]} />
        </div>
        <div className="flex h-[28rem] shrink-0 flex-col">
          {view === 'graph'
            ? <GraphView aria-label="Template graph" doc={doc} catalog={catalog} selected={null} onSelect={() => {}} readOnly />
            : <TextView doc={doc} onChange={readOnly ? undefined : setDoc} readOnly={readOnly} />}
        </div>
      </div>
    </div>
  );
}
