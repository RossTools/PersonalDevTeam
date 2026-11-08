import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { useDraft } from './useDraft';

const doc = (name: string): WorkflowDoc => ({ name, description: '', nodes: [], edges: [], settings: { maxSteps: 200, maxRuntimeMs: 1 } });

/** The hook over a saved version the test changes, as the workflow's poll would. */
function setup(saved = doc('v1'), key = 'k1') {
  return renderHook(({ s, k }: { s: WorkflowDoc; k: string }) => useDraft(s, k), { initialProps: { s: saved, k: key } });
}

describe('useDraft', () => {
  it('is dirty once edited and clean again when edited back; Discard returns to the saved version', () => {
    const { result } = setup();
    expect(result.current.dirty).toBe(false);
    act(() => result.current.set(doc('v2')));
    expect(result.current.dirty).toBe(true);
    // Equal after normalising (key order, nulls): not a change.
    act(() => result.current.set({ ...doc('v1'), description: '' }));
    expect(result.current.dirty).toBe(false);
    act(() => result.current.set(doc('v3')));
    act(() => result.current.discard());
    expect(result.current.doc.name).toBe('v1');
    expect(result.current.dirty).toBe(false);
    expect(result.current.canUndo).toBe(false);
  });

  it('undoes and redoes; a new edit drops what could be redone', () => {
    const { result } = setup();
    act(() => result.current.set(doc('a')));
    act(() => result.current.set(doc('b')));
    act(() => result.current.undo());
    expect(result.current.doc.name).toBe('a');
    expect(result.current.canRedo).toBe(true);
    act(() => result.current.redo());
    expect(result.current.doc.name).toBe('b');
    act(() => result.current.undo());
    act(() => result.current.undo());
    expect(result.current.doc.name).toBe('v1');
    expect(result.current.canUndo).toBe(false);
    // Nothing more to undo: stays.
    act(() => result.current.undo());
    expect(result.current.doc.name).toBe('v1');
    act(() => result.current.set(doc('c')));
    expect(result.current.canRedo).toBe(false);
    act(() => result.current.redo());
    expect(result.current.doc.name).toBe('c');
  });

  it('setting the same document is no step to undo', () => {
    const { result } = setup();
    const same = result.current.doc;
    act(() => result.current.set(same));
    expect(result.current.canUndo).toBe(false);
  });

  it('keeps 100 steps of undo', () => {
    const { result } = setup();
    for (let i = 0; i < 120; i++) act(() => result.current.set(doc(`e${i}`)));
    for (let i = 0; i < 100; i++) act(() => result.current.undo());
    expect(result.current.doc.name).toBe('e19');
    expect(result.current.canUndo).toBe(false);
  });

  it('a clean draft follows a new saved version, with a fresh history', () => {
    const { result, rerender } = setup();
    act(() => result.current.set(doc('a')));
    act(() => result.current.undo());
    rerender({ s: doc('v2'), k: 'k2' });
    expect(result.current.doc.name).toBe('v2');
    expect(result.current.savedElsewhere).toBe(false);
    expect(result.current.canRedo).toBe(false);
  });

  it('a dirty draft is never replaced: it hears the saved version changed, and Discard takes the new one', () => {
    const { result, rerender } = setup();
    act(() => result.current.set(doc('mine')));
    rerender({ s: doc('theirs'), k: 'k2' });
    expect(result.current.doc.name).toBe('mine');
    expect(result.current.savedElsewhere).toBe(true);
    expect(result.current.dirty).toBe(true);
    act(() => result.current.discard());
    expect(result.current.doc.name).toBe('theirs');
    expect(result.current.savedElsewhere).toBe(false);
    expect(result.current.dirty).toBe(false);
  });

  it('a dirty draft that matches the new saved version (its own save) becomes clean', () => {
    const { result, rerender } = setup();
    act(() => result.current.set(doc('mine')));
    rerender({ s: doc('mine'), k: 'k2' });
    expect(result.current.dirty).toBe(false);
    expect(result.current.savedElsewhere).toBe(false);
    expect(result.current.canUndo).toBe(false);
  });
});
