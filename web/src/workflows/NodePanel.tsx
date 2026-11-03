// The side panel for one node: its id and title, the config form from its kind's schema, the
// settings every node has, where its errors go, and its issues.
import { useEffect, useState } from 'react';
import { Trash2, X } from 'lucide-react';
import { Alert, Button, Field, IconButton, Input, SectionLabel, Select } from '@ui';
import type { Node } from '../api/gen/Node';
import type { WorkflowCheck } from '../api/gen/WorkflowCheck';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import {
  ERROR_PORT, configSchemaOf, connect, idProblem, issuesOf, kindOf, nodeTitle, referencePaths, removeNode, renameNode, updateNode, type Catalog,
} from './model';
import { SchemaForm } from './SchemaForm';

function NumberField({ label, value, onChange, hint, readOnly }: {
  label: string; value: number | null | undefined; onChange: (v: number | null) => void; hint?: string; readOnly?: boolean;
}) {
  return (
    <Field label={label} hint={hint}>
      <Input type="number" min={0} value={value == null ? '' : String(value)} readOnly={readOnly}
        onChange={(e) => onChange(e.target.value === '' ? null : Math.max(0, Math.trunc(Number(e.target.value)) || 0))} />
    </Field>
  );
}

/** The node's id: renamed when the field loses focus with a valid new id. */
function IdField({ doc, id, onRename, readOnly }: { doc: WorkflowDoc; id: string; onRename: (to: string) => void; readOnly?: boolean }) {
  const [text, setText] = useState(id);
  useEffect(() => setText(id), [id]);
  const problem = text === id ? null : idProblem(doc, id, text);
  const commit = () => {
    if (text !== id && !problem) onRename(text);
  };
  return (
    <Field label="Id" hint="Other steps use its output as {{steps.<id>}}." error={problem}>
      <Input mono value={text} readOnly={readOnly} onChange={(e) => setText(e.target.value.trim())} onBlur={commit}
        onKeyDown={(e) => { if (e.key === 'Enter') commit(); }} />
    </Field>
  );
}

export interface NodePanelProps {
  doc: WorkflowDoc;
  node: Node;
  catalog: Catalog;
  check?: WorkflowCheck | null;
  onChange: (doc: WorkflowDoc) => void;
  /** The node's id changed (rename) or it was removed (null). */
  onSelect: (id: string | null) => void;
  onClose: () => void;
  readOnly?: boolean;
}

export function NodePanel({ doc, node, catalog, check, onChange, onSelect, onClose, readOnly }: NodePanelProps) {
  const kind = kindOf(catalog, node.kind);
  const issues = issuesOf(check, node.id);
  const set = (change: (n: Node) => Node) => onChange(updateNode(doc, node.id, change));
  const errorEdge = doc.edges.find((e) => e.from === node.id && e.port === ERROR_PORT);
  const others = doc.nodes.filter((n) => n.id !== node.id && kindOf(catalog, n.kind)?.category !== 'trigger');
  const isTrigger = kind?.category === 'trigger';
  return (
    <section aria-label={`Node ${node.id}`} className="flex h-full min-h-0 flex-col">
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">{nodeTitle(node, catalog)}</h3>
          <p className="truncate font-mono text-meta text-muted-foreground">{node.kind}</p>
        </div>
        <IconButton label="Close the panel" size="sm" onClick={onClose}><X aria-hidden /></IconButton>
      </header>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
        {kind ? <p className="text-meta text-muted-foreground">{kind.description}</p> : <Alert tone="error">No node kind is called {node.kind}.</Alert>}
        {issues.map((i, n) => (
          <Alert key={n} tone={i.severity === 'error' ? 'error' : 'warning'}>{i.message}</Alert>
        ))}
        <IdField doc={doc} id={node.id} readOnly={readOnly} onRename={(to) => { onChange(renameNode(doc, node.id, to)); onSelect(to); }} />
        <Field label="Title" hint="Shown instead of the kind's name.">
          <Input value={node.title ?? ''} readOnly={readOnly} placeholder={kind?.title}
            onChange={(e) => set((n) => ({ ...n, title: e.target.value }))} />
        </Field>
        {kind && (
          <>
            <SectionLabel as="h4" className="pt-1">Settings</SectionLabel>
            <SchemaForm
              schema={configSchemaOf(catalog, node) ?? kind.configSchema}
              value={(node.config ?? {}) as Record<string, unknown>}
              onChange={(config) => set((n) => ({ ...n, config }))}
              refs={referencePaths(doc, catalog, node.id)}
              readOnly={readOnly}
            />
          </>
        )}
        {!isTrigger && (
          <>
            <SectionLabel as="h4" className="pt-1">When it fails</SectionLabel>
            <div className="grid grid-cols-2 gap-2">
              <NumberField label="Retries" readOnly={readOnly} value={node.retry?.max ?? null}
                onChange={(v) => set((n) => ({ ...n, retry: v ? { max: v, backoffMs: n.retry?.backoffMs ?? 5000 } : null }))} />
              <NumberField label="First wait (ms)" readOnly={readOnly} value={node.retry?.backoffMs ?? null} hint="Doubled each retry."
                onChange={(v) => set((n) => (n.retry ? { ...n, retry: { ...n.retry, backoffMs: v ?? 0 } } : n))} />
              <NumberField label="Time limit (ms)" readOnly={readOnly} value={node.timeoutMs}
                onChange={(v) => set((n) => ({ ...n, timeoutMs: v }))} />
              <NumberField label="Visits per run" readOnly={readOnly} value={node.maxVisits ?? null} hint="Loop limit; default 25."
                onChange={(v) => set((n) => ({ ...n, maxVisits: v }))} />
            </div>
            <Field label="On error, go to" hint="Without one, an error fails the run once its retries are spent.">
              <Select
                disabled={readOnly}
                options={[{ value: '', label: 'Fail the run' }, ...others.map((n) => ({ value: n.id, label: `${nodeTitle(n, catalog)} (${n.id})` }))]}
                value={errorEdge?.to ?? ''}
                onChange={(to) => {
                  const without = { ...doc, edges: doc.edges.filter((e) => !(e.from === node.id && e.port === ERROR_PORT)) };
                  onChange(to ? connect(without, { from: node.id, port: ERROR_PORT, to }) : without);
                }}
              />
            </Field>
          </>
        )}
        {!readOnly && (
          <Button variant="secondary" className="self-start" onClick={() => { onChange(removeNode(doc, node.id)); onSelect(null); }}>
            <Trash2 aria-hidden /> Remove node
          </Button>
        )}
      </div>
    </section>
  );
}
