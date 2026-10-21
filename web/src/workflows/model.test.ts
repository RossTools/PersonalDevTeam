import { describe, expect, it } from 'vitest';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import {
  addNode, canMove, connect, defaultConfig, freshId, layout, moveInChain, newNode, outline, referencePaths, removeNode,
  renameNode, summary, type Catalog,
} from './model';
import { CATALOG } from './testData';

const cat: Catalog = CATALOG;

const doc = (nodes: [string, string][], edges: [string, string, string][]): WorkflowDoc => ({
  name: 'w',
  description: '',
  nodes: nodes.map(([id, kind]) => ({ id, kind, version: 1, config: {} })),
  edges: edges.map(([from, port, to]) => ({ from, port, to })),
  settings: { maxSteps: 200, maxRuntimeMs: 1 },
});

describe('workflow model', () => {
  it('names new nodes after their kind and starts their config from the schema', () => {
    const d = doc([['send', 'session.send']], []);
    expect(freshId(d, 'session.send')).toBe('send_2');
    expect(freshId(d, 'gate')).toBe('gate');
    const n = newNode(d, cat.find((k) => k.id === 'gate')!);
    expect(n.config).toEqual({ question: '', choices: [] });
    expect(defaultConfig({ type: 'object', properties: { a: { type: 'boolean' }, b: { type: 'string' } }, required: ['a'] })).toEqual({ a: false });
  });

  it('adds a step between a port and what it led to, and removing it joins them again', () => {
    const d = doc([['t', 'manual'], ['e', 'end']], [['t', 'out', 'e']]);
    const added = addNode(d, { id: 's', kind: 'state.set', version: 1, config: {} }, { node: 't', port: 'out' }, 'out');
    expect(added.edges).toEqual([{ from: 't', port: 'out', to: 's' }, { from: 's', port: 'out', to: 'e' }]);
    expect(removeNode(added, 's').edges).toEqual([{ from: 't', port: 'out', to: 'e' }]);
    expect(connect(added, { from: 't', port: 'out', to: 's' })).toBe(added);
  });

  it('renames a node with its edges and step references', () => {
    const d = doc([['t', 'manual'], ['a', 'state.set'], ['b', 'end']], [['t', 'out', 'a'], ['a', 'out', 'b']]);
    d.nodes[2].config = { output: '{{steps.a.value}} {{ steps.a }} {{steps.ab}}' };
    const r = renameNode(d, 'a', 'x');
    expect(r.nodes.map((n) => n.id)).toEqual(['t', 'x', 'b']);
    expect(r.edges).toEqual([{ from: 't', port: 'out', to: 'x' }, { from: 'x', port: 'out', to: 'b' }]);
    expect(r.nodes[2].config).toEqual({ output: '{{steps.x.value}} {{ steps.x }} {{steps.ab}}' });
  });

  it('swaps a step with its neighbour in a chain', () => {
    const d = doc([['t', 'manual'], ['a', 'state.set'], ['b', 'ticket.move'], ['e', 'end']], [['t', 'out', 'a'], ['a', 'out', 'b'], ['b', 'out', 'e']]);
    expect(canMove(d, cat, 'a', 'up')).toBe(false);
    expect(canMove(d, cat, 'b', 'up')).toBe(true);
    expect(canMove(d, cat, 'a', 'down')).toBe(true);
    const m = moveInChain(d, cat, 'b', 'up', () => 'out');
    const order = (x: WorkflowDoc) => {
      const seq = ['t'];
      for (let id = 't'; ; ) {
        const e = x.edges.find((y) => y.from === id);
        if (!e) return seq;
        seq.push(e.to);
        id = e.to;
      }
    };
    expect(order(m)).toEqual(['t', 'b', 'a', 'e']);
    expect(order(moveInChain(m, cat, 'b', 'down', () => 'out'))).toEqual(['t', 'a', 'b', 'e']);
  });

  it('lays out nodes without a position by their depth', () => {
    const d = doc([['t', 'manual'], ['a', 'state.set'], ['b', 'end'], ['c', 'end']], [['t', 'out', 'a'], ['a', 'out', 'b'], ['a', 'out', 'c']]);
    const pos = layout(d, cat);
    expect(pos.get('t')).toEqual({ x: 0, y: 0 });
    expect(pos.get('a')!.x).toBeGreaterThan(0);
    expect(pos.get('b')!.x).toBe(pos.get('c')!.x);
    expect(pos.get('b')!.y).not.toBe(pos.get('c')!.y);
  });

  it("doesn't lay a node over one that has a position", () => {
    const d = doc([['t', 'manual'], ['a', 'state.set'], ['e', 'end']], [['t', 'out', 'a'], ['a', 'out', 'e']]);
    d.nodes[0].position = { x: 0, y: 0 };
    d.nodes[2].position = { x: 300, y: 0 };
    const pos = layout(d, cat);
    expect(pos.get('a')).toEqual({ x: 300, y: 150 });
  });

  it('shows a document as steps with branches, references for nodes seen before and loose nodes', () => {
    const d = doc(
      [['ci', 'ci.run_failed'], ['merged', 'attempt.merged'], ['if', 'condition'], ['tell', 'session.send'], ['end', 'end'], ['lost', 'end']],
      [['ci', 'out', 'if'], ['if', 'true', 'tell'], ['if', 'false', 'end'], ['merged', 'out', 'tell']],
    );
    const o = outline(d, cat);
    expect(o.roots.map((i) => [i.type, i.id])).toEqual([['node', 'ci'], ['node', 'if'], ['node', 'merged'], ['ref', 'tell']]);
    const cond = o.roots[1];
    expect(cond.type === 'node' && cond.branches.map((b) => [b.port, b.items.map((i) => i.id)])).toEqual([['true', ['tell']], ['false', ['end']]]);
    expect(o.loose.map((i) => i.id)).toEqual(['lost']);
  });

  it('sums up what enabling consents to and offers references', () => {
    const d = doc([['ci', 'ci.run_failed'], ['tell', 'session.send'], ['end', 'end']], [['ci', 'out', 'tell'], ['tell', 'sent', 'end']]);
    expect(summary(d, cat)).toEqual({ triggers: ['CI failed'], actions: ['Tell the agent'] });
    const refs = referencePaths(d, cat, 'end');
    expect(refs).toContain('trigger.pr');
    expect(refs).toContain('steps.tell');
    expect(refs).not.toContain('steps.ci');
  });
});
