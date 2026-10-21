import { useState } from 'react';
import { Bell, GitPullRequest, ShieldQuestion } from 'lucide-react';
import { FlowGraph, type FlowEdge, type FlowNode, type FlowSelection } from '@ui/FlowGraph';

const NODES: FlowNode[] = [
  { id: 'ci', title: 'CI failed', subtitle: 'ci.run_failed', icon: <GitPullRequest />, input: false, ports: [{ id: 'out', label: 'Then' }], position: { x: 0, y: 40 }, status: 'succeeded' },
  { id: 'ask', title: 'Ask the user', subtitle: 'gate', icon: <ShieldQuestion />, ports: [{ id: 'yes', label: 'yes' }, { id: 'no', label: 'no' }, { id: 'timeout', label: 'No answer in time' }], position: { x: 280, y: 0 }, status: 'waiting' },
  { id: 'tell', title: 'Tell the agent', subtitle: 'session.send', icon: <Bell />, ports: [{ id: 'sent', label: 'Sent' }], position: { x: 580, y: 0 }, issue: { severity: 'error', messages: ['tell: text is required'] } },
];

function Demo() {
  const [nodes, setNodes] = useState(NODES);
  const [edges, setEdges] = useState<FlowEdge[]>([{ from: 'ci', port: 'out', to: 'ask', taken: true }, { from: 'ask', port: 'yes', to: 'tell', label: 'yes' }]);
  const [selection, setSelection] = useState<FlowSelection>(null);
  return (
    <div className="h-96 rounded-md border border-border">
      <FlowGraph
        aria-label="Example workflow"
        nodes={nodes}
        edges={edges}
        selection={selection}
        onSelect={setSelection}
        onMove={(id, position) => setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, position } : n)))}
        onConnect={(e) => setEdges((es) => [...es, e])}
        onDelete={({ nodes: gone, edges: cut }) => {
          setNodes((ns) => ns.filter((n) => !gone.includes(n.id)));
          setEdges((es) => es.filter((e) => !gone.includes(e.from) && !gone.includes(e.to) && !cut.some((c) => c.from === e.from && c.port === e.port && c.to === e.to)));
        }}
      />
    </div>
  );
}

export const section = { title: 'Graph', order: 46, render: () => <Demo /> };
