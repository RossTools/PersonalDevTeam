// Dry run (Phase 7d spec §4): runs the draft against a recent event, a hand-written one or a manual input, changing
// nothing, and shows the path and what each step would do.
import { useMemo, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { Alert, Badge, Button, Dialog, Field, SegmentedControl, Select, Textarea } from '@ui';
import { api } from '../api/client';
import type { DryRunResult } from '../api/gen/DryRunResult';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { GraphView } from './GraphView';
import { deref, kindOf, triggerTypes, type Catalog } from './model';
import { RUN_STATUS, dryRunSteps, pathOf } from './runs';
import { StepList } from './StepList';

type Schema = Record<string, unknown>;

/** An example payload from a trigger's output schema: every required field with a placeholder. */
export function samplePayload(schema: Schema, root: Schema = schema, depth = 0): unknown {
  const s = deref(schema, root);
  if (Array.isArray(s.allOf)) return Object.assign({}, ...(s.allOf as Schema[]).map((x) => samplePayload(x, root, depth) as object));
  const variants = (s.anyOf ?? s.oneOf) as Schema[] | undefined;
  if (variants?.length) return samplePayload(variants.find((v) => deref(v, root).type !== 'null') ?? variants[0], root, depth);
  if (Array.isArray(s.enum)) return s.enum[0];
  if (s.const !== undefined) return s.const;
  const type = Array.isArray(s.type) ? s.type.find((t) => t !== 'null') : s.type;
  if (type === 'object' || s.properties) {
    if (depth > 3) return {};
    const props = (s.properties ?? {}) as Record<string, Schema>;
    return Object.fromEntries(Object.entries(props).map(([k, v]) => [k, samplePayload(v, root, depth + 1)]));
  }
  switch (type) {
    case 'string': return '';
    case 'integer': case 'number': return 0;
    case 'boolean': return false;
    case 'array': return [];
    default: return null;
  }
}

export function DryRunDialog({ open, onOpenChange, doc, workflowId, catalog }: {
  open: boolean; onOpenChange: (open: boolean) => void; doc: WorkflowDoc; workflowId?: string; catalog: Catalog;
}) {
  const types = triggerTypes(doc, catalog);
  const eventTypes = types.filter((t) => t !== 'manual');
  const [source, setSource] = useState<'recent' | 'write'>(eventTypes.length ? 'recent' : 'write');
  const [type, setType] = useState(types[0] ?? 'manual');
  const sample = useMemo(() => {
    const schema = (kindOf(catalog, type)?.outputSchema ?? {}) as Schema;
    return JSON.stringify(type === 'manual' ? {} : samplePayload(schema), null, 2);
  }, [catalog, type]);
  const [payload, setPayload] = useState<string | null>(null);
  const [eventId, setEventId] = useState<string | null>(null);
  const events = useQuery({
    queryKey: ['events', 'recent'],
    queryFn: () => api.listLogEvents({ limit: 200 }),
    enabled: open && source === 'recent',
  });
  const recent = (events.data ?? []).filter((e) => eventTypes.includes(e.type)).slice(0, 50);
  const text = payload ?? sample;
  let parsed: unknown = null;
  let parseError: string | null = null;
  try {
    parsed = JSON.parse(text);
  } catch (e) {
    parseError = (e as Error).message;
  }
  const run = useMutation({
    mutationFn: () => api.dryRunWorkflow({
      doc, workflowId: workflowId ?? null,
      event: source === 'write' ? { type, payload: parsed } : null,
      eventId: source === 'recent' ? eventId : null,
    }),
  });
  const result: DryRunResult | undefined = run.data;
  const steps = result?.run ? dryRunSteps(result.run.steps) : [];
  const ready = source === 'recent' ? !!eventId : !parseError;
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      size="xl"
      title="Dry run"
      description="Runs the workflow as it is in the editor against an event. Nothing is sent, moved or changed."
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>Close</Button>
          <Button variant="primary" onClick={() => run.mutate()} disabled={!ready} pending={run.isPending}>Dry run</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        {eventTypes.length > 0 && (
          <SegmentedControl aria-label="Event" value={source} onChange={setSource}
            options={[{ value: 'recent', label: 'A recent event' }, { value: 'write', label: 'Write one' }]} />
        )}
        {source === 'recent' ? (
          <Field label="Event" hint={events.isSuccess && recent.length === 0 ? `No recent ${eventTypes.join(', ')} events: write one instead.` : undefined}>
            <Select
              className="w-full"
              placeholder={events.isLoading ? 'Loading…' : 'Choose an event'}
              options={recent.map((e) => ({ value: e.id, label: `${e.type} · ${new Date(e.createdAt).toLocaleString()}${e.ticketId ? ` · ${String((e.payload as { ticketKey?: string })?.ticketKey ?? '')}` : ''}` }))}
              value={eventId}
              onChange={setEventId}
            />
          </Field>
        ) : (
          <>
            {types.length > 1 && (
              <Field label="Trigger">
                <Select options={types.map((t) => ({ value: t, label: t === 'manual' ? 'Run now (manual input)' : t }))} value={type}
                  onChange={(t) => { setType(t); setPayload(null); }} />
              </Field>
            )}
            <Field label={type === 'manual' ? 'Input' : 'Payload'} error={parseError} hint="JSON. {{trigger.x}} in the workflow reads these fields.">
              <Textarea mono rows={8} value={text} onChange={(e) => setPayload(e.target.value)} />
            </Field>
          </>
        )}
        {run.error && <Alert tone="error">{run.error.message}</Alert>}
        {result && result.issues.some((i) => i.severity === 'error') && (
          <Alert tone="error" title="The workflow has errors">{result.issues.filter((i) => i.severity === 'error').map((i) => i.message).join('; ')}</Alert>
        )}
        {result?.run && (
          <section aria-label="Dry run result" className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2 text-sm">
              {result.run.status ? <Badge tone={RUN_STATUS[result.run.status].tone}>Would end: {RUN_STATUS[result.run.status].text}</Badge>
                : <Badge tone="neutral">No trigger matched</Badge>}
              {result.run.triggers.map((t) => (
                <span key={t.node} className="text-meta text-muted-foreground">{t.node}: {t.matched ? 'matches' : `doesn't match${t.reason ? ` (${t.reason})` : ''}`}</span>
              ))}
            </div>
            {steps.length > 0 && (
              <div className="flex h-72 flex-col rounded-md border border-border">
                <GraphView aria-label="Dry run path" doc={doc} catalog={catalog} selected={null} onSelect={() => {}} readOnly path={pathOf(doc, steps)} />
              </div>
            )}
            <StepList doc={doc} catalog={catalog} steps={result.run.steps.map((s, i) => ({
              key: `${i}`, nodeId: s.nodeId, kind: s.kind, port: s.port, error: s.error, input: s.input, output: s.output,
              status: s.error ? { tone: 'failed', text: 'Error' } : { tone: 'accent', text: 'Ran' },
            }))} />
          </section>
        )}
      </div>
    </Dialog>
  );
}
