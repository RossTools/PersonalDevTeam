// A run's steps in order: status, the port taken, attempts, error, and the input and output as JSON.
import type { ReactNode } from 'react';
import { Badge, type BadgeTone } from '@ui';
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

function Json({ label, value }: { label: string; value: unknown }) {
  if (value === null || value === undefined || (typeof value === 'object' && Object.keys(value).length === 0)) return null;
  return (
    <details className="text-meta">
      <summary className="cursor-pointer text-muted-foreground">{label}</summary>
      <pre className="mt-1 max-h-60 overflow-auto rounded-md bg-muted p-2 font-mono whitespace-pre-wrap">{JSON.stringify(value, null, 2)}</pre>
    </details>
  );
}

export function StepList({ steps, doc, catalog }: { steps: StepRow[]; doc: WorkflowDoc; catalog: Catalog }) {
  if (!steps.length) return <p className="text-sm text-muted-foreground">No steps ran.</p>;
  return (
    <ol aria-label="Steps" className="flex flex-col gap-2">
      {steps.map((s, i) => {
        const node = doc.nodes.find((n) => n.id === s.nodeId);
        const would = s.output && typeof s.output === 'object' && 'would' in s.output ? String((s.output as { would: unknown }).would) : null;
        return (
          <li key={s.key} className="flex flex-col gap-1 rounded-md border border-border p-2">
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <span className="text-meta text-muted-foreground tabular-nums">{i + 1}</span>
              <span className="font-medium">{node ? nodeTitle(node, catalog) : s.nodeId}</span>
              <span className="font-mono text-meta text-muted-foreground">{s.nodeId} · {s.kind}</span>
              <Badge tone={s.status.tone}>{s.status.text}</Badge>
              {s.port && <span className="text-meta text-muted-foreground">→ {s.port}</span>}
              {s.attempts !== undefined && s.attempts > 1 && <span className="text-meta text-muted-foreground">{s.attempts} attempts</span>}
            </div>
            {would && <p className="text-sm">Would {would}</p>}
            {s.error && <p className="text-sm text-destructive">{s.error}</p>}
            {s.actions}
            <Json label="Input" value={s.input} />
            <Json label="Output" value={s.output} />
          </li>
        );
      })}
    </ol>
  );
}
