// A run's steps in order: status, the port taken, attempts, error, and the input and output as JSON.
import { useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { Badge, Disclosure, type BadgeTone } from '@ui';
import { cn } from '@ui/lib/utils';
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
 * `expandable` (phones, PDT-120): each step is a row to tap, which shows its input and output under it; otherwise
 * they are folded (a `Disclosure` each) in every step.
 */
export function StepList({ steps, doc, catalog, expandable = false }: { steps: StepRow[]; doc: WorkflowDoc; catalog: Catalog; expandable?: boolean }) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  if (!steps.length) return <p className="text-sm text-muted-foreground">No steps ran.</p>;
  return (
    <ol aria-label="Steps" className="flex flex-col gap-2">
      {steps.map((s, i) => {
        const node = doc.nodes.find((n) => n.id === s.nodeId);
        const would = s.output && typeof s.output === 'object' && 'would' in s.output ? String((s.output as { would: unknown }).would) : null;
        const shown = open.has(s.key);
        const head = (
          <>
              <span className="text-meta text-muted-foreground tabular-nums">{i + 1}</span>
              <span className="font-medium">{node ? nodeTitle(node, catalog) : s.nodeId}</span>
              <span className="font-mono text-meta text-muted-foreground">{s.nodeId} · {s.kind}</span>
              <Badge tone={s.status.tone}>{s.status.text}</Badge>
              {s.port && <span className="text-meta text-muted-foreground">→ {s.port}</span>}
              {s.attempts !== undefined && s.attempts > 1 && <span className="text-meta text-muted-foreground">{s.attempts} attempts</span>}
          </>
        );
        return (
          <li key={s.key} className="flex flex-col gap-1 rounded-md border border-border p-2">
            {expandable ? (
              <button type="button" aria-expanded={shown}
                onClick={() => setOpen((o) => { const n = new Set(o); if (n.has(s.key)) n.delete(s.key); else n.add(s.key); return n; })}
                className="-m-1 flex min-h-control-touch flex-wrap items-center gap-2 rounded-md p-1 text-left text-sm hover:bg-accent">
                <ChevronRight aria-hidden className={cn('size-4 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none', shown && 'rotate-90')} />
                {head}
              </button>
            ) : (
              <div className="flex flex-wrap items-center gap-2 text-sm">{head}</div>
            )}
            {would && <p className="text-sm">Would {would}</p>}
            {s.error && <p className="text-sm break-words text-destructive">{s.error}</p>}
            {s.actions}
            {expandable ? shown && (
              <>
                <JsonBlock label="Input" value={s.input} />
                <JsonBlock label="Output" value={s.output} />
              </>
            ) : (
              <>
                <Json label="Input" value={s.input} />
                <Json label="Output" value={s.output} />
              </>
            )}
          </li>
        );
      })}
    </ol>
  );
}
