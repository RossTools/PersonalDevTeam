// The text view's formats (Phase 7d spec §3.3): the document as JSON or TOML. TOML has no null, so null values are
// left out, which the document format reads the same.
import { parse as parseToml, stringify as stringifyToml } from 'smol-toml';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';

export type TextFormat = 'json' | 'toml';

function withoutNulls(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(withoutNulls);
  if (v && typeof v === 'object') {
    return Object.fromEntries(Object.entries(v).filter(([, x]) => x !== null && x !== undefined).map(([k, x]) => [k, withoutNulls(x)]));
  }
  return v;
}

/** TOML numbers can come back as bigint; the document wants plain numbers. */
function plain(v: unknown): unknown {
  if (typeof v === 'bigint') return Number(v);
  if (v instanceof Date) return v.toISOString();
  if (Array.isArray(v)) return v.map(plain);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, plain(x)]));
  return v;
}

export function toText(doc: WorkflowDoc, format: TextFormat): string {
  return format === 'json' ? `${JSON.stringify(doc, null, 2)}\n` : stringifyToml(withoutNulls(doc) as Record<string, unknown>);
}

/** The document the text holds, or why it doesn't hold one. */
export function fromText(text: string, format: TextFormat): { doc: WorkflowDoc } | { error: string } {
  let v: unknown;
  try {
    v = format === 'json' ? JSON.parse(text) : plain(parseToml(text));
  } catch (e) {
    return { error: (e as Error).message.split('\n')[0] };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) return { error: 'The document must be an object' };
  const d = v as Record<string, unknown>;
  if (typeof d.name !== 'string') return { error: 'name must be text' };
  if (!Array.isArray(d.nodes)) return { error: 'nodes must be a list' };
  for (const [i, n] of (d.nodes as unknown[]).entries()) {
    const node = n as Record<string, unknown>;
    if (!node || typeof node !== 'object' || typeof node.id !== 'string' || typeof node.kind !== 'string' || typeof node.version !== 'number') {
      return { error: `node ${i + 1} needs an id, a kind and a version` };
    }
  }
  if (d.edges !== undefined && !Array.isArray(d.edges)) return { error: 'edges must be a list' };
  const edges = ((d.edges ?? []) as Record<string, unknown>[]).map((e) => ({ ...e, port: typeof e.port === 'string' ? e.port : 'out' }));
  return {
    doc: {
      ...d,
      description: typeof d.description === 'string' ? d.description : '',
      nodes: (d.nodes as Record<string, unknown>[]).map((n) => ({ ...n, config: n.config ?? {} })),
      edges,
      settings: { maxSteps: 200, maxRuntimeMs: 604800000, ...(d.settings as object | undefined) },
    } as unknown as WorkflowDoc,
  };
}

/** Same document, ignoring nulls and key order (so a round trip through TOML is no change). */
export function sameDoc(a: WorkflowDoc, b: WorkflowDoc): boolean {
  const norm = (v: unknown): unknown => {
    const x = withoutNulls(v);
    if (Array.isArray(x)) return x.map(norm);
    if (x && typeof x === 'object') return Object.fromEntries(Object.keys(x).sort().map((k) => [k, norm((x as Record<string, unknown>)[k])]));
    return x;
  };
  return JSON.stringify(norm(a)) === JSON.stringify(norm(b));
}
