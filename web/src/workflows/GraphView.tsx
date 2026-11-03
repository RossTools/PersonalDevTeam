// The graph view: the document as nodes and edges in FlowGraph. Adding, connecting, moving and
// deleting edit the draft; a run's path can be laid over it.
import { useMemo } from 'react';
import { LayoutGrid, Plus } from 'lucide-react';
import { Button } from '@ui';
import { FlowGraph, type FlowEdge, type FlowNode, type FlowStatus, type FlowViewport } from '@ui/FlowGraph';
import type { WorkflowCheck } from '../api/gen/WorkflowCheck';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { readViewState, useViewKey, writeViewState } from '../common/viewState';
import { KindIcon, KindPicker } from './KindPicker';
import {
  ERROR_PORT, addNode, connect, disconnect, drawnPorts, edgeKey, issuesOf, kindOf, layout, moveNode, newNode, nodeTitle, portsOf,
  removeNode, spotAfter, type Catalog,
} from './model';
import { useCheck } from './useDraft';

/** A run (or dry run) drawn on the graph: each node's status and the edges taken. */
export interface PathOverlay {
  status: Map<string, FlowStatus>;
  taken: Set<string>;
}

export interface GraphViewProps {
  doc: WorkflowDoc;
  onChange?: (doc: WorkflowDoc) => void;
  catalog: Catalog;
  check?: WorkflowCheck | null;
  selected: string | null;
  onSelect: (id: string | null) => void;
  readOnly?: boolean;
  path?: PathOverlay;
  /** Names where the pan and zoom are kept (the view-state store), so the graph opens where it was left. */
  remember?: string;
  'aria-label'?: string;
}

export function GraphView({ doc, onChange, catalog, check: given, selected, onSelect, readOnly = false, path, remember, ...aria }: GraphViewProps) {
  const viewportKey = useViewKey(remember ?? '');
  // A graph shown without the builder's check (a run, a dry run, a template) checks its document itself, for the ports.
  const own = useCheck(doc, given === undefined).data;
  const check = given === undefined ? own : given;
  const edit = readOnly || !onChange ? undefined : onChange;
  const nodes = useMemo<FlowNode[]>(() => {
    const pos = layout(doc, catalog);
    return doc.nodes.map((n) => {
      const kind = kindOf(catalog, n.kind);
      const issues = issuesOf(check, n.id);
      const status = path?.status.get(n.id);
      return {
        id: n.id,
        title: nodeTitle(n, catalog),
        subtitle: n.id === n.kind ? n.kind : `${n.id} · ${n.kind}`,
        icon: <KindIcon category={kind?.category} />,
        input: kind?.category !== 'trigger',
        ports: drawnPorts(doc, n, catalog, check).map((p) => ({ id: p.id, label: p.title })),
        position: pos.get(n.id)!,
        ...(status ? { status } : {}),
        ...(issues.length
          ? { issue: { severity: issues.some((i) => i.severity === 'error') ? 'error' as const : 'warning' as const, messages: issues.map((i) => i.message) } }
          : {}),
      };
    });
  }, [doc, catalog, check, path]);
  const edges = useMemo<FlowEdge[]>(() => doc.edges.map((e) => {
    const from = doc.nodes.find((n) => n.id === e.from);
    const many = from ? drawnPorts(doc, from, catalog, check).length > 1 : false;
    const title = from ? drawnPorts(doc, from, catalog, check).find((p) => p.id === e.port)?.title : undefined;
    return {
      from: e.from, port: e.port, to: e.to,
      ...(many && title ? { label: title } : {}),
      ...(e.port === ERROR_PORT ? { dashed: true } : {}),
      ...(path?.taken.has(edgeKey(e.from, e.port, e.to)) ? { taken: true } : {}),
    };
  }), [doc, catalog, check, path]);

  const add = (kindId: string) => {
    if (!edit) return;
    const kind = kindOf(catalog, kindId)!;
    const from = selected ? doc.nodes.find((n) => n.id === selected) : undefined;
    const node = newNode(doc, kind, spotAfter(doc, catalog, from?.id));
    // Wired from the selected node's first port that leads nowhere yet.
    const open = from && kind.category !== 'trigger' ? portsOf(from, catalog, check).find((p) => !doc.edges.some((e) => e.from === from.id && e.port === p.id)) : undefined;
    edit(open ? addNode(doc, node, { node: from!.id, port: open.id }) : addNode(doc, node));
    onSelect(node.id);
  };

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {edit && (
        <div className="absolute top-2 left-2 z-(--pdt-z-popover) flex gap-1">
          <KindPicker catalog={catalog} onPick={(k) => add(k.id)} trigger={<Button size="sm"><Plus aria-hidden /> Add node</Button>} />
          <Button size="sm" variant="secondary" onClick={() => {
            const cleared = { ...doc, nodes: doc.nodes.map((n) => ({ ...n, position: null })) };
            const pos = layout(cleared, catalog);
            edit({ ...doc, nodes: doc.nodes.map((n) => ({ ...n, position: pos.get(n.id)! })) });
          }}>
            <LayoutGrid aria-hidden /> Arrange
          </Button>
        </div>
      )}
      <FlowGraph
        aria-label={aria['aria-label'] ?? 'Workflow graph'}
        className="min-h-0 flex-1"
        nodes={nodes}
        edges={edges}
        readOnly={!edit}
        selection={selected ? { kind: 'node', id: selected } : null}
        onSelect={(s) => onSelect(s?.kind === 'node' ? s.id : null)}
        {...(remember ? { viewport: readViewState<FlowViewport>(viewportKey), onViewportChange: (v: FlowViewport) => writeViewState(viewportKey, v) } : {})}
        onMove={(id, p) => edit?.(moveNode(doc, id, p))}
        onConnect={(e) => edit?.(connect(doc, e))}
        onDelete={({ nodes: gone, edges: cut }) => {
          if (!edit) return;
          let next = doc;
          for (const e of cut) next = disconnect(next, e);
          for (const id of gone) next = removeNode(next, id);
          edit(next);
          if (selected && gone.includes(selected)) onSelect(null);
        }}
      />
    </div>
  );
}
