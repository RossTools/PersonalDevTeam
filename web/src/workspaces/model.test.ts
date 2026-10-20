import { describe, expect, it } from 'vitest';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';
import { findTab, itemOf, nextNeedingYou, openTabBody, positionInAll, resolveCurrent, sameGroup, sameItem, type Item } from './model';

const tab = (id: string, workspaceId: string, item: Item, position = 0): Tab => ({
  id,
  workspaceId,
  sessionId: item.kind === 'session' ? item.id : null,
  runId: item.kind === 'run' ? item.id : null,
  ticketId: item.kind === 'ticket' ? item.id : null,
  position,
});
const ws = (id: string, tabs: Tab[], projectId: string | null = null): Workspace => ({
  id, name: id, projectId, position: 0, createdAt: 1, tabs,
});
const s = (id: string): Item => ({ kind: 'session', id });
const r = (id: string): Item => ({ kind: 'run', id });
const k = (id: string): Item => ({ kind: 'ticket', id });

describe('workspace model', () => {
  it('reads and compares tab items', () => {
    expect(itemOf(tab('t', 'w', s('a')))).toEqual(s('a'));
    expect(itemOf(tab('t', 'w', r('b')))).toEqual(r('b'));
    expect(sameItem(s('a'), s('a'))).toBe(true);
    expect(sameItem(s('a'), r('a'))).toBe(false);
  });

  it('handles ticket tabs', () => {
    expect(itemOf(tab('t', 'w', k('x')))).toEqual(k('x'));
    expect(sameItem(k('x'), s('x'))).toBe(false);
    const list = [ws('w1', [tab('t1', 'w1', s('a')), tab('t2', 'w1', k('x'))])];
    expect(findTab(list, k('x'))?.tab.id).toBe('t2');
    expect(openTabBody(k('x'))).toEqual({ ticketId: 'x' });
    expect(openTabBody(s('a'))).toEqual({ sessionId: 'a' });
    expect(openTabBody(r('b'))).toEqual({ runId: 'b' });
  });

  it('finds the tab and workspace holding an item', () => {
    const list = [ws('w1', [tab('t1', 'w1', s('a'))]), ws('w2', [tab('t2', 'w2', r('b'))])];
    expect(findTab(list, r('b'))?.tab.id).toBe('t2');
    expect(findTab(list, r('b'))?.workspace.id).toBe('w2');
    expect(findTab(list, s('zz'))).toBeNull();
  });

  it('resolves the current workspace and tab, falling back when they are gone', () => {
    const list = [ws('w1', [tab('t1', 'w1', s('a')), tab('t2', 'w1', s('b'))]), ws('w2', [])];
    expect(resolveCurrent(list, { workspaceId: 'w1', tabs: { w1: 't2' } })).toMatchObject({ workspace: { id: 'w1' }, tab: { id: 't2' } });
    expect(resolveCurrent(list, { workspaceId: 'gone', tabs: {} })).toMatchObject({ workspace: { id: 'w1' }, tab: { id: 't1' } });
    expect(resolveCurrent(list, { workspaceId: 'w1', tabs: { w1: 'gone' } }).tab?.id).toBe('t1');
    expect(resolveCurrent(list, { workspaceId: 'w2', tabs: {} })).toMatchObject({ workspace: { id: 'w2' }, tab: null });
    expect(resolveCurrent([], { workspaceId: null, tabs: {} })).toEqual({ workspace: null, tab: null });
  });

  it('cycles through the sessions that need you, starting after the current tab', () => {
    const list = [
      ws('w1', [tab('t1', 'w1', s('a')), tab('t2', 'w1', s('b'))]),
      ws('w2', [tab('t3', 'w2', s('c')), tab('t4', 'w2', r('d'))]),
    ];
    const needs = (item: Item) => item.kind === 'session' && ['a', 'c'].includes(item.id);
    expect(nextNeedingYou(list, 't1', needs)?.id).toBe('t3');
    expect(nextNeedingYou(list, 't3', needs)?.id).toBe('t1');
    expect(nextNeedingYou(list, null, needs)?.id).toBe('t1');
    expect(nextNeedingYou(list, 't1', () => false)).toBeNull();
  });
});

describe('ticket workspace groups', () => {
  const w = (id: string, ticket = false) => ({ id, name: id, projectId: null, position: 0, createdAt: 0, tabs: [],
    ...(ticket ? { ticket: { id: `t${id}`, projectId: 'p', key: `K-${id}`, title: id, stage: 'in_progress' as const, attemptId: 'a', attemptState: 'active' as const, pr: null, parts: null } } : {}) });
  const all = [w('p1'), w('t1', true), w('f1'), w('t2', true), w('t3', true)];

  it('splits ticket workspaces from the others', () => {
    expect(sameGroup(all, all[1]!).map((x) => x.id)).toEqual(['t1', 't2', 't3']);
    expect(sameGroup(all, all[0]!).map((x) => x.id)).toEqual(['p1', 'f1']);
  });

  it('maps a place in a group to a place in the full list', () => {
    const tickets = sameGroup(all, all[1]!);
    // t3 to the top of the tickets: just before t1.
    expect(positionInAll(all, tickets, 't3', 0)).toBe(1);
    // t1 to the end of the tickets: after t3.
    expect(positionInAll(all, tickets, 't1', 2)).toBe(4);
    // t1 one down: just before t3, which is at 3 once t1 is out.
    expect(positionInAll(all, tickets, 't1', 1)).toBe(3);
    const others = sameGroup(all, all[0]!);
    expect(positionInAll(all, others, 'f1', 0)).toBe(0);
  });
});
