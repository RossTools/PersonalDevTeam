import { describe, expect, it } from 'vitest';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';
import { findTab, itemOf, nextNeedingYou, resolveCurrent, sameItem, type Item } from './model';

const tab = (id: string, workspaceId: string, item: Item, position = 0): Tab => ({
  id,
  workspaceId,
  sessionId: item.kind === 'session' ? item.id : null,
  runId: item.kind === 'run' ? item.id : null,
  ticketId: null,
  position,
});
const ws = (id: string, tabs: Tab[], projectId: string | null = null): Workspace => ({
  id, name: id, projectId, position: 0, createdAt: 1, tabs,
});
const s = (id: string): Item => ({ kind: 'session', id });
const r = (id: string): Item => ({ kind: 'run', id });

describe('workspace model', () => {
  it('reads and compares tab items', () => {
    expect(itemOf(tab('t', 'w', s('a')))).toEqual(s('a'));
    expect(itemOf(tab('t', 'w', r('b')))).toEqual(r('b'));
    expect(sameItem(s('a'), s('a'))).toBe(true);
    expect(sameItem(s('a'), r('a'))).toBe(false);
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
