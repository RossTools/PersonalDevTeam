// A run's steps in order: status, the port taken, attempts, error, and the input and output as JSON.
import type { ReactNode } from 'react';
import { Badge, Disclosure, EmptyState, type BadgeTone } from '@ui';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { nodeTitle, type Catalog } from './model';

export interface StepRow {
  key: string;
  nodeId: string;
  kind: string;
  status: { tone: BadgeTone; text: string };
  port: string | null;
  error: string | null;
  input: unknown;
  output: unknown;
  attempts?: number;
  /** Extra controls (a waiting gate's choices). */
  actions?: ReactNode;
}

const empty = (value: unknown) => value === null || value === undefined || (typeof value === 'object' && Object.keys(value).length === 0);

function Json({ label, value }: { label: string; value: unknown }) {
  if (empty(value)) return null;
  return (
    <Disclosure title={label} className="text-meta">
      <pre className="max-h-60 overflow-auto rounded-md bg-muted p-2 font-mono whitespace-pre-wrap text-foreground">{JSON.stringify(value, null, 2)}</pre>
    </Disclosure>
  );
}

/** Shown open, on a phone: the input and output as JSON that wraps. */
function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="flex flex-col gap-1 text-meta">
      <span className="text-muted-foreground">{label}</span>
      {empty(value) ? <span className="text-muted-foreground">None</span>
        : <pre className="overflow-x-auto rounded-md bg-muted p-2 font-mono break-all whitespace-pre-wrap">{JSON.stringify(value, null, 2)}</pre>}
    </div>
  );
}

/**
 * `expandable` (phones, PDT-120): each step is a card to tap (a `Disclosure` card), which shows its input and output
 * under its error and actions; otherwise they are folded (a `Disclosure` each) in every step.
 */
export function StepList({ steps, doc, catalog, expandable = false }: { steps: StepRow[]; doc: WorkflowDoc; catalog: Catalog; expandable?: boolean }) {
  if (!steps.length) return <EmptyState size="inline">No steps ran.</EmptyState>;
  return (
    <ol aria-label="Steps" className="flex flex-col gap-2">
      {steps.map((s, i) => {
        const node = doc.nodes.find((n) => n.id === s.nodeId);
        const would = s.output && typeof s.output === 'object' && 'would' in s.output ? String((s.output as { would: unknown }).would) : null;
        const head = (
          <span className="flex flex-wrap items-center gap-2 font-normal">
            <span className="text-meta text-muted-foreground tabular-nums">{i + 1}</span>
            <span className="font-medium">{node ? nodeTitle(node, catalog) : s.nodeId}</span>
            <span className="font-mono text-meta text-muted-foreground">{s.nodeId} · {s.kind}</span>
            <Badge tone={s.status.tone}>{s.status.text}</Badge>
            {s.port && <span className="text-meta text-muted-foreground">→ {s.port}</span>}
            {s.attempts !== undefined && s.attempts > 1 && <span className="text-meta text-muted-foreground">{s.attempts} attempts</span>}
          </span>
        );
        const notes = (would || s.error || s.actions) && (
          <>
            {would && <p className="text-sm">Would {would}</p>}
            {s.error && <p className="text-sm break-words text-destructive">{s.error}</p>}
            {s.actions}
          </>
        );
        return (
          <li key={s.key}>
            {expandable ? (
              <Disclosure
                variant="card"
                title={head}
                triggerClassName="px-2 py-2"
                panelClassName="flex flex-col gap-1 px-2 pb-2"
                visible={notes && <div className="flex flex-col gap-1 px-2 pb-2">{notes}</div>}
              >
                <JsonBlock label="Input" value={s.input} />
                <JsonBlock label="Output" value={s.output} />
              </Disclosure>
            ) : (
              <div className="flex flex-col gap-1 rounded-md border border-border p-2">
                <div className="text-sm">{head}</div>
                {notes}
                <Json label="Input" value={s.input} />
                <Json label="Output" value={s.output} />
              </div>
            )}
          </li>
        );
      })}
    </ol>
  );
}
