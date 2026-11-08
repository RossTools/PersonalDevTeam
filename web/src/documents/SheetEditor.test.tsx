import { act, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import { renderWithQuery, stubApi } from '../test-utils';
import type { DocEvent } from './useDocumentEvents';
import SheetEditor from './SheetEditor';

// IronCalc's WASM model and canvas widget stand in as a fake: each load makes a model, the widget counts its mounts
// (a redraw), and the event stream is the handler the editor gives useDocumentEvents.
const fake = vi.hoisted(() => ({
  models: [] as { bytes: Uint8Array; applied: number[][]; queue: Uint8Array; selected: [number, number] | null }[],
  mounts: 0,
  onEvent: null as ((e: DocEvent) => void) | null,
  onOpen: null as (() => void) | null,
}));

vi.mock('@ironcalc/workbook', () => {
  class Model {
    applied: number[][] = [];
    queue = new Uint8Array(0);
    selected: [number, number] | null = null;
    constructor(public bytes: Uint8Array) {}
    static fromBytes(bytes: Uint8Array) {
      const m = new Model(bytes);
      fake.models.push(m);
      return m;
    }
    getSelectedView() {
      return { sheet: 0, row: 3, column: 2, range: [3, 2, 3, 2], top_row: 1, left_column: 1 };
    }
    getWorksheetsProperties() {
      return [{ name: 'Sheet1' }];
    }
    getFormattedCellValue() {
      return '';
    }
    setSelectedSheet() {}
    setSelectedCell(row: number, col: number) {
      this.selected = [row, col];
    }
    flushSendQueue() {
      const q = this.queue;
      this.queue = new Uint8Array(0);
      return q;
    }
    applyExternalDiffs(d: Uint8Array) {
      if (d[0] === 0xff) throw new Error('bad diffs');
      this.applied.push([...d]);
    }
  }
  function IronCalc() {
    useEffect(() => {
      fake.mounts++;
    }, []);
    return <div data-testid="grid" />;
  }
  return { Model, IronCalc, init: async () => {} };
});

vi.mock('./useDocumentEvents', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./useDocumentEvents')>()),
  useDocumentEvents: (_id: string, enabled: boolean, onEvent: (e: DocEvent) => void, onOpen?: () => void) => {
    fake.onEvent = enabled ? onEvent : null;
    fake.onOpen = enabled ? (onOpen ?? null) : null;
  },
}));

const b64 = (...bytes: number[]) => btoa(String.fromCharCode(...bytes));
const send = (e: DocEvent) => act(() => fake.onEvent!(e));
const model = (i = -1) => fake.models.at(i)!;

let sheetSeq = 4;
let sheet: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  fake.models = [];
  fake.mounts = 0;
  fake.onEvent = null;
  sheetSeq = 4;
  stubApi({ 'GET /api/documents/d1/charts': [] });
  sheet = vi.spyOn(api, 'documentSheet').mockImplementation(async () => ({ bytes: new Uint8Array([sheetSeq]), seq: sheetSeq }));
});

afterEach(() => vi.restoreAllMocks());

async function open() {
  renderWithQuery(<SheetEditor id="d1" />);
  await screen.findByTestId('grid');
  await waitFor(() => expect(fake.onEvent).not.toBeNull());
}

describe('SheetEditor live updates', () => {
  it("applies the next change in order and redraws; skips one it already has (its own, sent back)", async () => {
    await open();
    const mounts = fake.mounts;
    send({ type: 'diffs', seq: 5, from: 'agent', data: b64(1, 2) });
    expect(model().applied).toEqual([[1, 2]]);
    await waitFor(() => expect(fake.mounts).toBe(mounts + 1));
    // Seq 5 again (or an older one): already applied, nothing changes.
    send({ type: 'diffs', seq: 5, from: 'agent', data: b64(9) });
    send({ type: 'diffs', seq: 3, from: 'agent', data: b64(9) });
    expect(model().applied).toEqual([[1, 2]]);
    expect(sheet).toHaveBeenCalledTimes(1);
  });

  it("skips the change its own edit made once pdtd numbered it", async () => {
    const diffs = vi.spyOn(api, 'sendSheetDiffs').mockResolvedValue({ seq: 5, conflict: false });
    await open();
    model().queue = new Uint8Array([7, 7, 7]);
    await waitFor(() => expect(diffs).toHaveBeenCalledWith('d1', expect.any(String), 4, new Uint8Array([7, 7, 7])));
    // Its own edit, as change 5: not applied twice. The next one is.
    send({ type: 'diffs', seq: 5, from: 'me', data: b64(7) });
    expect(model().applied).toEqual([]);
    send({ type: 'diffs', seq: 6, from: 'agent', data: b64(3) });
    expect(model().applied).toEqual([[3]]);
    expect(sheet).toHaveBeenCalledTimes(1);
  });

  it('loads the sheet again when it missed a change, keeping the selected cell', async () => {
    await open();
    sheetSeq = 9;
    send({ type: 'diffs', seq: 7, from: 'agent', data: b64(1) });
    await waitFor(() => expect(fake.models).toHaveLength(2));
    expect(model(0).applied).toEqual([]);
    expect(model().bytes).toEqual(new Uint8Array([9]));
    expect(model().selected).toEqual([3, 2]);
    // The new model is at seq 9: the next change is 10.
    send({ type: 'diffs', seq: 9, from: 'agent', data: b64(1) });
    send({ type: 'diffs', seq: 10, from: 'agent', data: b64(2) });
    expect(model().applied).toEqual([[2]]);
  });

  it("loads the sheet again when a change won't apply", async () => {
    await open();
    send({ type: 'diffs', seq: 5, from: 'agent', data: b64(0xff) });
    await waitFor(() => expect(fake.models).toHaveLength(2));
  });

  it('reloads on a newer reload, ignores an older one, and catches up on reconnect', async () => {
    await open();
    send({ type: 'reload', seq: 4 });
    expect(sheet).toHaveBeenCalledTimes(1);
    sheetSeq = 6;
    send({ type: 'reload', seq: 6 });
    await waitFor(() => expect(fake.models).toHaveLength(2));
    act(() => fake.onOpen!());
    await waitFor(() => expect(fake.models).toHaveLength(3));
  });

  it('sends its own edits before reloading, so they are not lost', async () => {
    const diffs = vi.spyOn(api, 'sendSheetDiffs').mockResolvedValue({ seq: 5, conflict: false });
    await open();
    model().queue = new Uint8Array([1, 1]);
    sheetSeq = 8;
    send({ type: 'reload', seq: 8 });
    await waitFor(() => expect(fake.models).toHaveLength(2));
    expect(diffs).toHaveBeenCalledTimes(1);
    expect(diffs.mock.invocationCallOrder[0]!).toBeLessThan(sheet.mock.invocationCallOrder[1]!);
  });

  it('says so and reloads when its edit crossed someone else’s', async () => {
    vi.spyOn(api, 'sendSheetDiffs').mockResolvedValue({ seq: 6, conflict: true });
    await open();
    sheetSeq = 6;
    model().queue = new Uint8Array([1, 1]);
    expect(await screen.findByText(/Someone else changed this sheet at the same time/)).toBeInTheDocument();
    await waitFor(() => expect(fake.models).toHaveLength(2));
  });

  it('says so and shows what is saved when its edit could not be sent', async () => {
    vi.spyOn(api, 'sendSheetDiffs').mockRejectedValue(new Error('offline'));
    await open();
    model().queue = new Uint8Array([1, 1]);
    expect(await screen.findByText(/Your last change could not be saved \(offline\)/)).toBeInTheDocument();
    await waitFor(() => expect(fake.models).toHaveLength(2));
  });
});
