// The workflow document as the builder edits it (Phase 7d spec §3): pure functions over a WorkflowDoc, shared by the
// graph, outline and text views. Every edit returns a new document.
import type { Edge } from '../api/gen/Edge';
import type { KindSpec } from '../api/gen/KindSpec';
import type { Node } from '../api/gen/Node';
import type { PortSpec } from '../api/gen/PortSpec';
import type { WorkflowCheck } from '../api/gen/WorkflowCheck';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';

export const ERROR_PORT = 'error';

export type Catalog = KindSpec[];

export function kindOf(catalog: Catalog, kind: string): KindSpec | undefined {
  return catalog.find((k) => k.id === kind);
}

export function isTrigger(catalog: Catalog, node: Node): boolean {
  return kindOf(catalog, node.kind)?.category === 'trigger';
}

/** The node's ports: from the latest check (dynamic ports follow the config), else the kind's static ones. */
export function portsOf(node: Node, catalog: Catalog, check?: WorkflowCheck | null): PortSpec[] {
  const ports = check?.ports[node.id] ?? kindOf(catalog, node.kind)?.ports ?? [];
  return ports;
}

/** The ports to draw: the node's, plus `error` when an edge leaves by it. */
export function drawnPorts(doc: WorkflowDoc, node: Node, catalog: Catalog, check?: WorkflowCheck | null): PortSpec[] {
  const ports = portsOf(node, catalog, check);
  return hasErrorPath(doc, node.id) ? [...ports, { id: ERROR_PORT, title: 'On error' }] : ports;
}

export function hasErrorPath(doc: WorkflowDoc, id: string): boolean {
  return doc.edges.some((e) => e.from === id && e.port === ERROR_PORT);
}

export function nodeTitle(node: Node, catalog: Catalog): string {
  return node.title || kindOf(catalog, node.kind)?.title || node.id;
}

const sameEdge = (a: Edge, b: Edge) => a.from === b.from && a.port === b.port && a.to === b.to;

/** An id no node has yet, from the kind's last segment (`session.send` → `send`, `send_2`). */
export function freshId(doc: WorkflowDoc, kind: string): string {
  const base = (kind.split('.').pop() || 'node').replace(/[^A-Za-z0-9_]/g, '_');
  const taken = new Set(doc.nodes.map((n) => n.id));
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base}_${i}`)) return `${base}_${i}`;
}

type Schema = Record<string, unknown>;

/** A JSON Schema's `$ref` resolved against the root's `$defs` / `definitions`. */
export function deref(schema: Schema | undefined, root: Schema): Schema {
  let s = schema ?? {};
  for (let i = 0; i < 10 && typeof s.$ref === 'string'; i++) {
    const name = s.$ref.split('/').pop()!;
    const defs = (root.$defs ?? root.definitions ?? {}) as Record<string, Schema>;
    s = { ...defs[name], ...Object.fromEntries(Object.entries(s).filter(([k]) => k !== '$ref')) };
  }
  return s;
}

/** A starting value for a required field of this schema. */
function emptyValue(schema: Schema, root: Schema): unknown {
  const s = deref(schema, root);
  if ('default' in s) return s.default;
  if (Array.isArray(s.enum) && s.enum.length) return s.enum[0];
  if (typeof s.const !== 'undefined') return s.const;
  const variants = (s.anyOf ?? s.oneOf) as Schema[] | undefined;
  if (variants?.length) return emptyValue(variants[0], root);
  const type = Array.isArray(s.type) ? s.type.find((t) => t !== 'null') : s.type;
  switch (type) {
    case 'string': return '';
    case 'boolean': return false;
    case 'integer': case 'number': return 0;
    case 'array': return [];
    case 'object': return defaultConfig(s, root);
    default: return null;
  }
}

/** A new node's config: its kind's required fields with empty values. */
export function defaultConfig(schema: Schema, root: Schema = schema): Record<string, unknown> {
  const props = (deref(schema, root).properties ?? {}) as Record<string, Schema>;
  const required = (deref(schema, root).required ?? []) as string[];
  return Object.fromEntries(required.filter((k) => k in props).map((k) => [k, emptyValue(props[k], root)]));
}

/** A new node of `kind`; an entry that presets another kind (an MCP tool's) makes that kind's node, titled after it. */
export function newNode(doc: WorkflowDoc, kind: KindSpec, position?: { x: number; y: number }): Node {
  const preset = kind.preset;
  if (preset) {
    const args = ((kind.configSchema.properties ?? {}) as Record<string, Schema>).arguments;
    const tool = typeof preset.config.tool === 'string' ? preset.config.tool : preset.kind;
    return {
      id: freshId(doc, tool),
      kind: preset.kind,
      version: kind.version,
      title: kind.title,
      config: { ...preset.config, ...(args ? { arguments: defaultConfig(args) } : {}) },
      ...(position ? { position } : {}),
    } as Node;
  }
  return {
    id: freshId(doc, kind.id),
    kind: kind.id,
    version: kind.version,
    config: defaultConfig(kind.configSchema),
    ...(position ? { position } : {}),
  } as Node;
}

/** The schema of a node's settings: its kind's, or for an MCP tool node, its tool's entry (the tool's arguments). */
export function configSchemaOf(catalog: Catalog, node: Node): Record<string, unknown> | undefined {
  const c = (node.config ?? {}) as Record<string, unknown>;
  if (node.kind === 'mcp.tool' && typeof c.server === 'string' && typeof c.tool === 'string') {
    const entry = kindOf(catalog, `mcp:${c.server}/${c.tool}`);
    if (entry) return entry.configSchema;
  }
  return kindOf(catalog, node.kind)?.configSchema;
}

export function connect(doc: WorkflowDoc, e: Edge): WorkflowDoc {
  if (e.from === e.to || doc.edges.some((x) => sameEdge(x, e))) return doc;
  return { ...doc, edges: [...doc.edges, e] };
}

export function disconnect(doc: WorkflowDoc, e: Edge): WorkflowDoc {
  return { ...doc, edges: doc.edges.filter((x) => !sameEdge(x, e)) };
}

/** Adds `node`; with `after`, between that node's port and what the port led to (the new node's first port). */
export function addNode(doc: WorkflowDoc, node: Node, after?: { node: string; port: string }, firstPort?: string): WorkflowDoc {
  let next: WorkflowDoc = { ...doc, nodes: [...doc.nodes, node] };
  if (!after) return next;
  const moved = doc.edges.filter((e) => e.from === after.node && e.port === after.port);
  next = { ...next, edges: next.edges.filter((e) => !moved.includes(e)) };
  next = connect(next, { from: after.node, port: after.port, to: node.id });
  if (firstPort) for (const e of moved) next = connect(next, { from: node.id, port: firstPort, to: e.to });
  return next;
}

/** Removes a node and its edges; one way in and one way out are joined, so a chain stays a chain. */
export function removeNode(doc: WorkflowDoc, id: string): WorkflowDoc {
  const ins = doc.edges.filter((e) => e.to === id && e.from !== id);
  const outs = doc.edges.filter((e) => e.from === id && e.to !== id && e.port !== ERROR_PORT);
  let next: WorkflowDoc = {
    ...doc,
    nodes: doc.nodes.filter((n) => n.id !== id),
    edges: doc.edges.filter((e) => e.from !== id && e.to !== id),
  };
  if (ins.length === 1 && outs.length === 1) next = connect(next, { from: ins[0].from, port: ins[0].port, to: outs[0].to });
  return next;
}

export function updateNode(doc: WorkflowDoc, id: string, change: (n: Node) => Node): WorkflowDoc {
  return { ...doc, nodes: doc.nodes.map((n) => (n.id === id ? change(n) : n)) };
}

export function moveNode(doc: WorkflowDoc, id: string, position: { x: number; y: number }): WorkflowDoc {
  return updateNode(doc, id, (n) => ({ ...n, position }));
}

const ID = /^[A-Za-z_][A-Za-z0-9_-]*$/;

/** Why `to` can't be the node's new id, or null. */
export function idProblem(doc: WorkflowDoc, from: string, to: string): string | null {
  if (!ID.test(to)) return 'Letters, digits, - and _, starting with a letter';
  if (to !== from && doc.nodes.some((n) => n.id === to)) return `A node is already called ${to}`;
  return null;
}

/** Renames a node: its edges and the `{{steps.<id>}}` references to it follow. */
export function renameNode(doc: WorkflowDoc, from: string, to: string): WorkflowDoc {
  if (from === to) return doc;
  const ref = new RegExp(`(\\{\\{\\s*steps\\.)${from.replace(/[-]/g, '\\-')}(?=[.\\s}])`, 'g');
  const rewrite = (v: unknown): unknown =>
    typeof v === 'string' ? v.replace(ref, `$1${to}`)
      : Array.isArray(v) ? v.map(rewrite)
        : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, rewrite(x)])) : v;
  return {
    ...doc,
    nodes: doc.nodes.map((n) => ({ ...(n.id === from ? { ...n, id: to } : n), config: rewrite(n.config) as Node['config'] })),
    edges: doc.edges.map((e) => ({ ...e, from: e.from === from ? to : e.from, to: e.to === from ? to : e.to })),
  };
}

/** The one node leading into `id` by its only way in, when that node has only that way out. */
function chainBefore(doc: WorkflowDoc, catalog: Catalog, id: string): Edge | null {
  const ins = doc.edges.filter((e) => e.to === id);
  if (ins.length !== 1) return null;
  const prev = doc.nodes.find((n) => n.id === ins[0].from);
  if (!prev || isTrigger(catalog, prev)) return null;
  const prevOuts = doc.edges.filter((e) => e.from === prev.id);
  return prevOuts.length === 1 ? ins[0] : null;
}

/** Whether `id` can swap with the step before it (`up`) or after it. */
export function canMove(doc: WorkflowDoc, catalog: Catalog, id: string, dir: 'up' | 'down'): boolean {
  if (dir === 'up') return movable(doc, catalog, id) && chainBefore(doc, catalog, id) !== null && movable(doc, catalog, chainBefore(doc, catalog, id)!.from);
  const outs = doc.edges.filter((e) => e.from === id);
  return outs.length === 1 && canMove(doc, catalog, outs[0].to, 'up');
}

/** A node that can change places in a chain: not a trigger, and leaving by one port. */
function movable(doc: WorkflowDoc, catalog: Catalog, id: string): boolean {
  const n = doc.nodes.find((x) => x.id === id);
  if (!n || isTrigger(catalog, n)) return false;
  return new Set(doc.edges.filter((e) => e.from === id).map((e) => e.port)).size <= 1;
}

/** Swaps `id` with the step before it (`up`) or after it, in a chain Q → P → id → … */
export function moveInChain(doc: WorkflowDoc, catalog: Catalog, id: string, dir: 'up' | 'down', portOf: (id: string) => string): WorkflowDoc {
  if (dir === 'down') {
    const out = doc.edges.find((e) => e.from === id);
    return out && canMove(doc, catalog, out.to, 'up') ? moveInChain(doc, catalog, out.to, 'up', portOf) : doc;
  }
  if (!canMove(doc, catalog, id, 'up')) return doc;
  const pe = chainBefore(doc, catalog, id)!;
  const p = pe.from;
  const intoP = doc.edges.filter((e) => e.to === p);
  const outOfId = doc.edges.filter((e) => e.from === id);
  const pPort = pe.port;
  const idPort = outOfId[0]?.port ?? portOf(id);
  const touched = (e: Edge) => e.to === p || e.from === p || e.from === id;
  const edges = doc.edges.filter((e) => !touched(e));
  edges.push(...intoP.map((e) => ({ ...e, to: id })));
  edges.push({ from: id, port: idPort, to: p });
  edges.push(...outOfId.map((e) => ({ from: p, port: pPort, to: e.to })));
  // Swap the positions too, so the graph reads in the new order.
  const pos = (x: string) => doc.nodes.find((n) => n.id === x)?.position;
  const [pp, ip] = [pos(p), pos(id)];
  const nodes = doc.nodes.map((n) => (n.id === p && ip ? { ...n, position: ip } : n.id === id && pp ? { ...n, position: pp } : n));
  return { ...doc, nodes, edges };
}

export const COLUMN = 300;
export const ROW = 150;

/** Positions for nodes that have none: a column per step from the triggers, a row per node in it. */
export function layout(doc: WorkflowDoc, catalog: Catalog): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  const depth = new Map<string, number>();
  const queue = doc.nodes.filter((n) => isTrigger(catalog, n) || !doc.edges.some((e) => e.to === n.id)).map((n) => n.id);
  queue.forEach((id) => depth.set(id, 0));
  while (queue.length) {
    const id = queue.shift()!;
    for (const e of doc.edges.filter((x) => x.from === id)) {
      if (!depth.has(e.to)) {
        depth.set(e.to, depth.get(id)! + 1);
        queue.push(e.to);
      }
    }
  }
  const rows = new Map<number, number>();
  for (const n of doc.nodes) if (n.position) out.set(n.id, n.position);
  // A node without a position goes in its column, below anything already there.
  const taken = (x: number, y: number) => [...out.values()].some((p) => Math.abs(p.x - x) < COLUMN / 2 && Math.abs(p.y - y) < ROW / 2);
  for (const n of doc.nodes) {
    if (n.position) continue;
    const d = depth.get(n.id) ?? 0;
    let r = rows.get(d) ?? 0;
    while (taken(d * COLUMN, r * ROW)) r++;
    rows.set(d, r + 1);
    out.set(n.id, { x: d * COLUMN, y: r * ROW });
  }
  return out;
}

/** Where a node added after `from` goes: right of it, below anything already there. */
export function spotAfter(doc: WorkflowDoc, catalog: Catalog, from?: string): { x: number; y: number } {
  const pos = layout(doc, catalog);
  if (!from || !pos.has(from)) {
    const maxY = Math.max(-ROW, ...[...pos.values()].map((p) => p.y));
    return { x: 0, y: maxY + ROW };
  }
  const base = pos.get(from)!;
  const x = base.x + COLUMN;
  let y = base.y;
  while ([...pos.values()].some((p) => Math.abs(p.x - x) < COLUMN / 2 && Math.abs(p.y - y) < ROW / 2)) y += ROW;
  return { x, y };
}

// ---- outline ----

export type OutlineItem =
  | { type: 'node'; id: string; branches: OutlineBranch[]; path: string }
  /** A node shown in full elsewhere. */
  | { type: 'ref'; id: string; path: string };

export interface OutlineBranch {
  /** The node and port it leaves by. */
  from: string;
  port: string;
  label: string;
  items: OutlineItem[];
}

export interface Outline {
  roots: OutlineItem[];
  /** Nodes no trigger leads to. */
  loose: OutlineItem[];
}

/**
 * The document as ordered steps: each trigger, then what its ports lead to. A node with one port and one next step
 * continues the same list; otherwise each port is a branch. A node reached again is a reference to where it is shown.
 */
export function outline(doc: WorkflowDoc, catalog: Catalog, check?: WorkflowCheck | null): Outline {
  const shown = new Set<string>();
  const seq = (start: string, path: string): OutlineItem[] => {
    const items: OutlineItem[] = [];
    let id: string | null = start;
    let i = 0;
    while (id) {
      const p = `${path}.${i++}`;
      if (shown.has(id)) {
        items.push({ type: 'ref', id, path: p });
        break;
      }
      shown.add(id);
      const node = doc.nodes.find((n) => n.id === id);
      if (!node) break;
      const ports = drawnPorts(doc, node, catalog, check);
      const outs = doc.edges.filter((e) => e.from === id);
      const only = ports.length === 1 ? outs.filter((e) => e.port === ports[0].id) : [];
      if (ports.length === 1 && only.length === 1 && outs.length === 1) {
        items.push({ type: 'node', id, branches: [], path: p });
        id = only[0].to;
        continue;
      }
      const branches: OutlineBranch[] = ports.map((port) => ({
        from: node.id,
        port: port.id,
        label: port.title,
        items: outs.filter((e) => e.port === port.id).flatMap((e, j) => seq(e.to, `${p}.${port.id}.${j}`)),
      }));
      items.push({ type: 'node', id, branches, path: p });
      id = null;
    }
    return items;
  };
  const triggers = doc.nodes.filter((n) => isTrigger(catalog, n));
  const roots = triggers.flatMap((t, i) => seq(t.id, `r${i}`));
  const loose = doc.nodes.filter((n) => !shown.has(n.id)).flatMap((n, i) => (shown.has(n.id) ? [] : seq(n.id, `l${i}`)));
  return { roots, loose };
}

/** The node an outline row leads on to by its only port (for "Add step after"). */
export function onlyPort(doc: WorkflowDoc, node: Node, catalog: Catalog, check?: WorkflowCheck | null): string | null {
  const ports = portsOf(node, catalog, check);
  return ports.length === 1 ? ports[0].id : null;
}

/** What enabling consents to: the triggers it runs on and the actions it can take, by title. */
export function summary(doc: WorkflowDoc, catalog: Catalog): { triggers: string[]; actions: string[] } {
  const titles = (cat: (c: string | undefined) => boolean) =>
    [...new Set(doc.nodes.filter((n) => cat(kindOf(catalog, n.kind)?.category)).map((n) => kindOf(catalog, n.kind)?.title ?? n.kind))];
  return {
    triggers: titles((c) => c === 'trigger'),
    actions: titles((c) => c === 'action' || c === 'gate'),
  };
}

/** The event types a document triggers on (for the dry run's event picker). */
export function triggerTypes(doc: WorkflowDoc, catalog: Catalog): string[] {
  return [...new Set(doc.nodes.filter((n) => isTrigger(catalog, n)).map((n) => n.kind))];
}

/** `{{…}}` paths a node's text fields can use: the triggers' fields and the steps before it. */
export function referencePaths(doc: WorkflowDoc, catalog: Catalog, nodeId: string): string[] {
  const out = new Set<string>(['trigger.ticketId', 'trigger.attemptId', 'trigger.projectId', 'run.id']);
  for (const t of doc.nodes.filter((n) => isTrigger(catalog, n))) {
    const schema = (kindOf(catalog, t.kind)?.outputSchema ?? {}) as Schema;
    for (const p of schemaPaths(schema, schema, 'trigger', 0)) out.add(p);
  }
  const before = new Set<string>();
  const walk = (id: string) => {
    for (const e of doc.edges.filter((x) => x.to === id)) {
      if (!before.has(e.from)) {
        before.add(e.from);
        walk(e.from);
      }
    }
  };
  walk(nodeId);
  for (const id of before) {
    const n = doc.nodes.find((x) => x.id === id);
    if (n && !isTrigger(catalog, n)) out.add(`steps.${id}`);
  }
  return [...out].sort();
}

function schemaPaths(schema: Schema, root: Schema, prefix: string, depth: number): string[] {
  const s = deref(schema, root);
  const props = (s.properties ?? {}) as Record<string, Schema>;
  const flat = Object.entries(props).flatMap(([k, v]) => {
    const p = `${prefix}.${k}`;
    const inner = deref(v, root);
    const nested = depth < 2 && inner.properties ? schemaPaths(inner, root, p, depth + 1) : [];
    return [p, ...nested];
  });
  // Flattened structs (`#[serde(flatten)]`) come as allOf.
  const all = ((s.allOf ?? []) as Schema[]).flatMap((x) => schemaPaths(x, root, prefix, depth));
  return [...flat, ...all];
}

/** The issues for one node, worst first. */
export function issuesOf(check: WorkflowCheck | null | undefined, id: string) {
  return (check?.issues ?? []).filter((i) => i.node === id);
}

export function hasErrors(check: WorkflowCheck | null | undefined): boolean {
  return (check?.issues ?? []).some((i) => i.severity === 'error');
}

/** The document with positions filled in for every node (so a move in the graph doesn't jumble the rest). */
export function withLayout(doc: WorkflowDoc, catalog: Catalog): WorkflowDoc {
  if (doc.nodes.every((n) => n.position)) return doc;
  const pos = layout(doc, catalog);
  return { ...doc, nodes: doc.nodes.map((n) => (n.position ? n : { ...n, position: pos.get(n.id)! })) };
}
