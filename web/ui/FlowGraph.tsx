import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  ReactFlowProvider,
  applyNodeChanges,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/base.css';
import './FlowGraph.css';
import { cn } from '@ui/lib/utils';

/** A node's state in a run: the ring around its card. */
export type FlowStatus = 'running' | 'waiting' | 'failed' | 'succeeded' | 'skipped';

export interface FlowPort {
  id: string;
  label: string;
}

export interface FlowNode {
  id: string;
  title: string;
  /** Under the title: the kind. */
  subtitle?: string;
  icon?: ReactNode;
  /** false for nodes nothing leads into (triggers). */
  input?: boolean;
  /** One handle each, on the right. */
  ports: FlowPort[];
  position: { x: number; y: number };
  status?: FlowStatus;
  /** Validation problems: the worst severity and the messages. */
  issue?: { severity: 'error' | 'warning'; messages: string[] };
}

export interface FlowEdge {
  from: string;
  port: string;
  to: string;
  /** The path a run took. */
  taken?: boolean;
  /** Shown on the edge (the port, when a node has several). */
  label?: string;
  /** Drawn dashed (an error path). */
  dashed?: boolean;
}

export type FlowSelection = { kind: 'node'; id: string } | { kind: 'edge'; edge: FlowEdge } | null;

export interface FlowGraphProps {
  nodes: FlowNode[];
  edges: FlowEdge[];
  selection?: FlowSelection;
  onSelect?: (s: FlowSelection) => void;
  /** A node dropped somewhere new. */
  onMove?: (id: string, position: { x: number; y: number }) => void;
  /** A port dragged to a node. */
  onConnect?: (edge: { from: string; port: string; to: string }) => void;
  /** Delete/Backspace on the selection. */
  onDelete?: (what: { nodes: string[]; edges: FlowEdge[] }) => void;
  /** No dragging, connecting or deleting (a run's path, a repo workflow). */
  readOnly?: boolean;
  'aria-label'?: string;
  className?: string;
}

const edgeId = (e: { from: string; port: string; to: string }) => `${e.from}\u0000${e.port}\u0000${e.to}`;

const ring: Record<FlowStatus, string> = {
  running: 'ring-2 ring-running',
  waiting: 'ring-2 ring-waiting',
  failed: 'ring-2 ring-failed',
  succeeded: 'ring-2 ring-running/60',
  skipped: 'opacity-60',
};

type CardData = FlowNode & Record<string, unknown>;

const Card = memo(function Card({ data, selected }: NodeProps<Node<CardData>>) {
  return (
    <div
      data-testid={`flow-node-${data.id}`}
      data-status={data.status}
      className={cn(
        'min-w-44 max-w-64 rounded-md border bg-card text-card-foreground shadow-popover',
        selected ? 'border-primary' : 'border-border',
        data.status && ring[data.status],
      )}
    >
      {data.input !== false && <Handle type="target" position={Position.Left} className="top-5!" />}
      <div className="flex items-start gap-2 px-2.5 py-2">
        {data.icon && <span className="mt-0.5 flex shrink-0 text-muted-foreground [&_svg]:size-4">{data.icon}</span>}
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{data.title}</div>
          {data.subtitle && <div className="truncate text-meta text-muted-foreground">{data.subtitle}</div>}
        </div>
        {data.issue && (
          <span
            role="img"
            aria-label={`${data.issue.severity === 'error' ? 'Error' : 'Warning'}: ${data.issue.messages.join('; ')}`}
            title={data.issue.messages.join('\n')}
            className={cn('mt-1 size-2 shrink-0 rounded-full', data.issue.severity === 'error' ? 'bg-failed' : 'bg-waiting')}
          />
        )}
      </div>
      {data.ports.length > 0 && (
        <div className="border-t border-border py-1">
          {data.ports.map((p) => (
            <div key={p.id} className="relative flex justify-end py-0.5 pr-3 pl-2 text-meta text-muted-foreground">
              <span className="truncate">{p.label}</span>
              <Handle type="source" id={p.id} position={Position.Right} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
});

const nodeTypes = { card: Card };

function Graph({ nodes, edges, selection, onSelect, onMove, onConnect, onDelete, readOnly = false, className, ...aria }: FlowGraphProps) {
  const selectedNode = selection?.kind === 'node' ? selection.id : null;
  const selectedEdge = selection?.kind === 'edge' ? edgeId(selection.edge) : null;
  const incoming = useMemo<Node<CardData>[]>(
    () => nodes.map((n) => ({ id: n.id, type: 'card', position: n.position, data: n as CardData, selected: n.id === selectedNode })),
    [nodes, selectedNode],
  );
  // React Flow moves nodes itself while they're dragged; the props take over again when the drag ends.
  const [local, setLocal] = useState(incoming);
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setLocal(incoming);
  }, [incoming]);
  const byId = useMemo(() => new Map(edges.map((e) => [edgeId(e), e])), [edges]);
  const flowEdges = useMemo<Edge[]>(
    () => edges.map((e) => ({
      id: edgeId(e),
      source: e.from,
      sourceHandle: e.port,
      target: e.to,
      selected: edgeId(e) === selectedEdge,
      className: cn(e.taken && 'pdt-taken', e.dashed && 'pdt-error'),
      markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
      ...(e.label ? { label: e.label } : {}),
    })),
    [edges, selectedEdge],
  );
  const onNodesChange = (changes: NodeChange<Node<CardData>>[]) => {
    // Removing and selecting are the parent's: it hears about them through onDelete and onSelect.
    const kept = changes.filter((c) => c.type !== 'remove' && c.type !== 'select');
    if (kept.length) setLocal((ns) => applyNodeChanges(kept, ns));
  };
  return (
    <ReactFlow<Node<CardData>>
      {...aria}
      className={cn('pdt-flow', className)}
      nodes={local}
      edges={flowEdges}
      nodeTypes={nodeTypes}
      onNodesChange={onNodesChange}
      onNodeDragStart={() => { dragging.current = true; }}
      onNodeDragStop={(_, n) => {
        dragging.current = false;
        onMove?.(n.id, { x: Math.round(n.position.x), y: Math.round(n.position.y) });
      }}
      onNodeClick={(_, n) => onSelect?.({ kind: 'node', id: n.id })}
      onEdgeClick={(_, e) => { const fe = byId.get(e.id); if (fe) onSelect?.({ kind: 'edge', edge: fe }); }}
      onPaneClick={() => onSelect?.(null)}
      onConnect={(c) => {
        if (c.source && c.target && c.sourceHandle && c.source !== c.target) onConnect?.({ from: c.source, port: c.sourceHandle, to: c.target });
      }}
      onDelete={({ nodes: ns, edges: es }) => {
        const removed = { nodes: ns.map((n) => n.id), edges: es.flatMap((e) => byId.get(e.id) ?? []) };
        if (removed.nodes.length || removed.edges.length) onDelete?.(removed);
      }}
      nodesDraggable={!readOnly}
      nodesConnectable={!readOnly}
      elementsSelectable
      deleteKeyCode={readOnly ? null : ['Backspace', 'Delete']}
      fitView
      fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
      minZoom={0.2}
      proOptions={{ hideAttribution: true }}
    >
      <Background gap={20} size={1} />
      <Controls showInteractive={false} />
    </ReactFlow>
  );
}

/**
 * A node graph (React Flow): cards with an input on the left and a handle per port on the right, edges between them.
 * Controlled: the parent owns the nodes, edges and selection and hears about moves, new edges and deletions. Features
 * never import @xyflow/react themselves.
 */
export function FlowGraph(props: FlowGraphProps) {
  return (
    <ReactFlowProvider>
      <Graph {...props} />
    </ReactFlowProvider>
  );
}
