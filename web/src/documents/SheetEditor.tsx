// A local sheet in IronCalc's spreadsheet editor (PDT-315), loaded only when a sheet opens (its WASM is large). pdtd
// holds the sheet's live model: this view loads it, sends its own edits as IronCalc diffs, and applies everyone
// else's (an agent's) from the document's event stream, in order.
import '@ironcalc/workbook/style.css';
import { IronCalc, Model, init } from '@ironcalc/workbook';
import { ArcElement, BarController, BarElement, CategoryScale, Chart, Filler, Legend, LineController, LineElement, LinearScale, PieController, PointElement, ScatterController, Title, Tooltip } from 'chart.js';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ArrowDownAZ, ArrowUpZA, ChartColumn, Filter, FilterX } from 'lucide-react';
import { Alert, Button, Dialog, Field, Input, MenuButton, Select, Skeleton, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { ChartKind } from '../api/gen/ChartKind';
import type { ChartSpec } from '../api/gen/ChartSpec';
import type { ChartView } from '../api/gen/ChartView';
import type { CommentAnchor } from '../api/gen/CommentAnchor';
import type { DocumentEdit } from '../api/gen/DocumentEdit';
import { actionError, runAction } from '../common/useAction';
import { cssVar } from '../terminal/theme';
import { documentKeys, useDocumentCharts } from './queries';
import { fromBase64, useDocumentEvents, VIEW_ID } from './useDocumentEvents';

Chart.register(BarController, BarElement, LineController, LineElement, PointElement, PieController, ArcElement, ScatterController, CategoryScale, LinearScale, Filler, Legend, Title, Tooltip);

let wasm: Promise<unknown> | null = null;
const ready = () => (wasm ??= init());

/** How often the view's own edits go to pdtd. */
const SEND_EVERY = 200;

/** The sheet's colours and type from PDT's tokens (the widget draws on a canvas, so they are read as values). */
function themeVariables() {
  return {
    '--typography-font-family': cssVar('--pdt-font-sans'),
    '--palette-common-white': cssVar('--pdt-surface'),
    '--palette-common-black': cssVar('--pdt-fg'),
    '--palette-primary-main': cssVar('--pdt-accent'),
    '--palette-primary-light': cssVar('--pdt-accent-soft'),
    '--palette-primary-dark': cssVar('--pdt-accent'),
    '--palette-primary-contrast-text': cssVar('--pdt-on-accent'),
    '--palette-error-main': cssVar('--pdt-failed'),
    '--palette-warning-main': cssVar('--pdt-waiting'),
    '--palette-success-main': cssVar('--pdt-running'),
    '--palette-grey-50': cssVar('--pdt-bg'),
    '--palette-grey-100': cssVar('--pdt-surface-2'),
    '--palette-grey-200': cssVar('--pdt-border'),
    '--palette-grey-300': cssVar('--pdt-border'),
    '--palette-grey-500': cssVar('--pdt-muted'),
    '--palette-grey-600': cssVar('--pdt-muted'),
    '--palette-grey-700': cssVar('--pdt-muted'),
    '--palette-grey-800': cssVar('--pdt-fg'),
    '--palette-grey-900': cssVar('--pdt-fg'),
    '--palette-surface-elevated': cssVar('--pdt-surface'),
    '--palette-surface-elevated-border': cssVar('--pdt-border'),
    '--palette-input-background': cssVar('--pdt-surface'),
    '--palette-input-border': cssVar('--pdt-input'),
    '--palette-sheet-header-corner-background': cssVar('--pdt-surface-2'),
    '--palette-sheet-header-text-color': cssVar('--pdt-muted'),
    '--palette-sheet-header-background': cssVar('--pdt-bg'),
    '--palette-sheet-header-global-selector-color': cssVar('--pdt-muted'),
    '--palette-sheet-header-selected-background': cssVar('--pdt-accent-soft'),
    '--palette-sheet-header-full-selected-background': cssVar('--pdt-accent'),
    '--palette-sheet-header-selected-color': cssVar('--pdt-fg'),
    '--palette-sheet-header-border-color': cssVar('--pdt-border'),
    '--palette-sheet-grid-color': cssVar('--pdt-border'),
    '--palette-sheet-grid-separator-color': cssVar('--pdt-input'),
    '--palette-sheet-default-text-color': cssVar('--pdt-fg'),
    '--palette-sheet-outline-color': cssVar('--pdt-accent'),
    '--palette-sheet-outline-editing-color': cssVar('--pdt-accent'),
    '--palette-sheet-outline-background-color': cssVar('--pdt-accent-soft'),
  };
}

/** The widget, drawn again only when `mount` changes (re-rendering it while a cell is being typed in loses the typing). */
const Grid = memo(function Grid({ model, mount }: { model: Model; mount: number }) {
  const theme = useMemo(() => themeVariables(), []);
  return <IronCalc key={mount} model={model} themeVariables={theme} />;
});

const letters = (c: number): string => (c <= 26 ? String.fromCharCode(64 + c) : letters(Math.floor((c - 1) / 26)) + letters(((c - 1) % 26) + 1));
const cellName = (row: number, col: number) => `${letters(col)}${row}`;

/** The selection now: its sheet's name and range (1-based rows and columns, in order). */
function selectionOf(model: Model) {
  const v = model.getSelectedView();
  const [a, b, c, d] = v.range;
  const name = model.getWorksheetsProperties()[v.sheet]?.name ?? 'Sheet1';
  return { sheet: name, index: v.sheet, r1: Math.min(a, c), c1: Math.min(b, d), r2: Math.max(a, c), c2: Math.max(b, d), row: v.row, column: v.column };
}

/** Where a cell's top-left corner is drawn on the grid's canvas, as the widget lays it out (its row and column
 *  headers are 28 and 30 pixels), or null when it is scrolled out of view above or to the left. */
function cellPosition(model: Model, sheet: number, row: number, col: number): { x: number; y: number } | null {
  const v = model.getSelectedView();
  const frozenRows = model.getFrozenRowsCount(sheet);
  const frozenCols = model.getFrozenColumnsCount(sheet);
  let y = 28;
  for (let r = 1; r <= Math.min(frozenRows, row - 1); r++) y += model.getRowHeight(sheet, r);
  if (row > frozenRows) {
    if (row < v.top_row) return null;
    for (let r = Math.max(v.top_row, frozenRows + 1); r < row; r++) y += model.getRowHeight(sheet, r);
  }
  let x = 30;
  for (let c = 1; c <= Math.min(frozenCols, col - 1); c++) x += model.getColumnWidth(sheet, c);
  if (col > frozenCols) {
    if (col < v.left_column) return null;
    for (let c = Math.max(v.left_column, frozenCols + 1); c < col; c++) x += model.getColumnWidth(sheet, c);
  }
  return { x, y };
}

/** Where a chart goes when its spec doesn't say: two columns right of its data, level with its top. */
function anchorFor(data: string): string {
  const [from, to = from] = data.split(':');
  const p = parseCell(from ?? ''), q = parseCell(to ?? '');
  return p && q ? cellName(Math.min(p.row, q.row), Math.max(p.col, q.col) + 2) : 'A1';
}

function parseCell(a: string): { row: number; col: number } | null {
  const m = /^([A-Z]{1,3})(\d+)$/i.exec(a.trim());
  if (!m) return null;
  const col = [...m[1]!.toUpperCase()].reduce((n, ch) => n * 26 + ch.charCodeAt(0) - 64, 0);
  return { row: Number(m[2]), col };
}

/** A chart's size on the grid (pixels). */
const CHART_SIZE = { width: 480, height: 300 };

const KINDS: { value: ChartKind; label: string }[] = [
  { value: 'column', label: 'Column' },
  { value: 'bar', label: 'Bar' },
  { value: 'line', label: 'Line' },
  { value: 'area', label: 'Area' },
  { value: 'pie', label: 'Pie' },
  { value: 'scatter', label: 'Scatter' },
];

function SheetChart({ view, onEdit }: { view: ChartView; onEdit: (e: DocumentEdit) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const chart = useRef<{ destroy(): void } | null>(null);
  const { spec, id } = view.chart;
  useEffect(() => {
    const colors = [1, 2, 3, 4, 5, 6].map((i) => cssVar(`--pdt-chart-${i}`));
    const text = cssVar('--pdt-fg');
    const grid = cssVar('--pdt-border');
    const pie = spec.kind === 'pie';
    const datasets = view.series.map((s, i) => {
      const data = s.values;
      const color = colors[i % colors.length]!;
      if (spec.kind === 'scatter') {
        return { label: s.name, data: data.map((y, j) => ({ x: Number(view.labels[j]) || j + 1, y: y ?? Number.NaN })), backgroundColor: color, borderColor: color };
      }
      return {
        label: s.name,
        data: data.map((v) => v ?? Number.NaN),
        backgroundColor: pie ? data.map((_, j) => colors[j % colors.length]!) : color,
        borderColor: pie ? cssVar('--pdt-surface') : color,
        fill: spec.kind === 'area',
      };
    });
    const type = spec.kind === 'area' ? 'line' : spec.kind === 'column' ? 'bar' : spec.kind;
    const config = {
      type,
      data: { labels: view.labels, datasets },
      options: {
        // A bar chart's bars lie across; a column chart's stand up.
        indexAxis: spec.kind === 'bar' ? 'y' : 'x',
        responsive: true,
        maintainAspectRatio: false,
        animation: { duration: 150 },
        color: text,
        plugins: { title: { display: !!spec.title, text: spec.title ?? '', color: text }, legend: { labels: { color: text } } },
        scales: pie ? {} : { x: { ticks: { color: text }, grid: { color: grid } }, y: { ticks: { color: text }, grid: { color: grid } } },
      },
    };
    chart.current?.destroy();
    // Chart.js's typing wants one chart type per config; the spec picks it at run time.
    chart.current = new Chart(canvas.current!, config as unknown as ConstructorParameters<typeof Chart>[1]);
    return () => {
      chart.current?.destroy();
      chart.current = null;
    };
  }, [view, spec]);
  const entries = (): MenuEntry[] => [
    ...KINDS.filter((k) => k.value !== spec.kind).map((k) => ({ label: `Show as ${k.label.toLowerCase()}`, onSelect: () => onEdit({ op: 'update_chart', id, chart: { ...spec, kind: k.value } }) })),
    { separator: true },
    { label: 'Remove chart', danger: true, onSelect: () => onEdit({ op: 'delete_chart', id }) },
  ];
  return (
    <figure aria-label={spec.title || 'Chart'} className="relative h-full w-full rounded-md border border-border bg-card p-2 shadow-card">
      <canvas ref={canvas} />
      <MenuButton label="Chart actions" entries={entries} placement="bottom end" className="absolute top-1 right-1" />
    </figure>
  );
}

export interface SheetEditorProps {
  id: string;
  /** Where the editor tells the page how to read its selection (comments go on it) and show a range. */
  onReady?: (api: { selection: () => CommentAnchor | null; show: (a: CommentAnchor) => void; sheet: () => string } | null) => void;
}

/**
 * The live sheet: IronCalc's editor (formulas, formats, several sheets) with PDT's charts floating over it at their
 * cells, and sort and filter. Another view's or an agent's change shows at once; when two land together, the last one
 * wins and a notice says the sheet now shows both.
 */
export default function SheetEditor({ id, onReady }: SheetEditorProps) {
  const qc = useQueryClient();
  const [model, setModel] = useState<Model | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Bumped to redraw the widget after outside changes (the 0.9 widget has no refresh; a remount keeps the selection
  // and scroll, which live in the model).
  const [mount, setMount] = useState(0);
  const [charting, setCharting] = useState(false);
  const seq = useRef(0);
  const modelRef = useRef<Model | null>(null);
  const host = useRef<HTMLDivElement>(null);
  const sending = useRef<Promise<void> | null>(null);
  const redrawPending = useRef(false);
  const charts = useDocumentCharts(id, model !== null);
  const refreshCharts = useCallback(() => void qc.invalidateQueries({ queryKey: documentKeys.charts(id) }), [qc, id]);

  const load = useCallback(async () => {
    await ready();
    const { bytes, seq: s } = await api.documentSheet(id);
    const m = Model.fromBytes(bytes, 'en');
    const old = modelRef.current;
    if (old) {
      // Keep where the user was.
      const v = old.getSelectedView();
      try {
        m.setSelectedSheet(v.sheet);
        m.setSelectedCell(v.row, v.column);
      } catch {
        // That sheet or cell is gone.
      }
    }
    seq.current = s;
    modelRef.current = m;
    setModel(m);
    setMount((n) => n + 1);
    refreshCharts();
  }, [id, refreshCharts]);

  useEffect(() => {
    load().catch((e: unknown) => setError(actionError(e)));
  }, [load]);

  /** Sends the view's edits not sent yet. */
  const flush = useCallback(async () => {
    const m = modelRef.current;
    if (!m) return;
    await sending.current;
    const diffs = m.flushSendQueue();
    if (diffs.length <= 1) return;
    const p = (async () => {
      try {
        const r = await api.sendSheetDiffs(id, VIEW_ID, seq.current, diffs);
        seq.current = r.seq;
        refreshCharts();
        if (r.conflict) {
          setNotice('Someone else changed this sheet at the same time. It now shows both changes.');
          await load();
        }
      } catch (e) {
        setNotice(`Your last change could not be saved (${actionError(e)}). The sheet shows what is saved.`);
        await load();
      }
    })();
    sending.current = p;
    await p;
  }, [id, load, refreshCharts]);

  useEffect(() => {
    if (!model) return;
    const t = setInterval(() => void flush(), SEND_EVERY);
    return () => clearInterval(t);
  }, [model, flush]);

  /** Whether the user is typing into a cell (a redraw would lose it). */
  const editing = () => {
    const a = document.activeElement;
    return !!a && !!host.current?.contains(a) && (a instanceof HTMLTextAreaElement || a instanceof HTMLInputElement) && a.value !== '';
  };
  const redraw = useCallback(() => {
    if (editing()) {
      redrawPending.current = true;
      return;
    }
    redrawPending.current = false;
    setMount((n) => n + 1);
  }, []);
  useEffect(() => {
    const t = setInterval(() => {
      if (redrawPending.current) redraw();
    }, 300);
    return () => clearInterval(t);
  }, [redraw]);

  useDocumentEvents(id, model !== null, (e) => {
    const m = modelRef.current;
    if (!m) return;
    if (e.type === 'diffs') {
      if (e.seq <= seq.current) return;
      if (e.seq !== seq.current + 1) {
        void flush().then(load);
        return;
      }
      try {
        m.applyExternalDiffs(fromBase64(e.data));
        seq.current = e.seq;
        redraw();
        refreshCharts();
      } catch {
        void load();
      }
    } else if (e.type === 'reload') {
      if (e.seq > seq.current) void flush().then(load);
    } else if (e.type === 'charts') {
      refreshCharts();
    } else if (e.type === 'comments') {
      void qc.invalidateQueries({ queryKey: documentKeys.comments(id) });
    }
  }, () => void flush().then(load));

  // The page reads the selection (for comments) and asks to show a comment's cells.
  useEffect(() => {
    if (!model) return;
    onReady?.({
      selection: () => {
        const s = selectionOf(model);
        const range = s.r1 === s.r2 && s.c1 === s.c2 ? cellName(s.r1, s.c1) : `${cellName(s.r1, s.c1)}:${cellName(s.r2, s.c2)}`;
        return { on: 'cell', sheet: s.sheet, range };
      },
      show: (a) => {
        if (a.on !== 'cell') return;
        const i = model.getWorksheetsProperties().findIndex((p) => p.name === a.sheet);
        const [from, to = from] = a.range.split(':');
        const p = parseCell(from ?? ''), q = parseCell(to ?? '');
        if (i < 0 || !p || !q) return;
        model.setSelectedSheet(i);
        model.setSelectedCell(p.row, p.col);
        model.setSelectedRange(p.row, p.col, q.row, q.col);
        setMount((n) => n + 1);
      },
      sheet: () => selectionOf(model).sheet,
    });
    return () => onReady?.(null);
  }, [model, onReady]);

  const edit = (edits: DocumentEdit[]) => void runAction(async () => {
    await flush();
    await api.editDocument(id, edits);
    refreshCharts();
  });

  // Charts follow the grid as it scrolls and its columns change size.
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!model) return;
    const t = setInterval(() => setTick((n) => n + 1), 150);
    return () => clearInterval(t);
  }, [model]);

  if (error) return <Alert tone="error">The sheet could not be opened: {error}</Alert>;
  if (!model) return <Skeleton lines={8} label="Opening the sheet" />;

  const sel = selectionOf(model);
  const sortRange = sel.r1 !== sel.r2 ? `${cellName(sel.r1, sel.c1)}:${cellName(sel.r2, sel.c2)}` : '';
  const col = letters(sel.column);
  const value = model.getFormattedCellValue(sel.index, sel.row, sel.column);
  const canvasEl = host.current?.querySelector<HTMLCanvasElement>('.ic-worksheet-sheet-canvas');
  const hostBox = host.current?.getBoundingClientRect();
  const gridBox = canvasEl?.getBoundingClientRect();
  const shown = (charts.data ?? []).filter((c) => c.sheet === sel.sheet);

  return (
    <div className="flex min-h-0 flex-col gap-2">
      <div role="toolbar" aria-label="Sheet tools" className="flex flex-wrap items-center gap-1">
        <Button variant="ghost" size="sm" compact onClick={() => edit([{ op: 'sort', sheet: sel.sheet, range: sortRange, by: [{ column: col, descending: false }], header: sortRange === '' }])}>
          <ArrowDownAZ aria-hidden /> Sort A→Z by {col}
        </Button>
        <Button variant="ghost" size="sm" compact onClick={() => edit([{ op: 'sort', sheet: sel.sheet, range: sortRange, by: [{ column: col, descending: true }], header: sortRange === '' }])}>
          <ArrowUpZA aria-hidden /> Z→A
        </Button>
        <Button variant="ghost" size="sm" compact disabled={!value} onClick={() => edit([{ op: 'filter', sheet: sel.sheet, range: sortRange, column: col, when: { type: 'equal_to', value } }])}>
          <Filter aria-hidden /> Only rows with “{value.length > 16 ? `${value.slice(0, 16)}…` : value}” in {col}
        </Button>
        <Button variant="ghost" size="sm" compact onClick={() => edit([{ op: 'clear_filter', sheet: sel.sheet }])}>
          <FilterX aria-hidden /> Show all rows
        </Button>
        <Button variant="ghost" size="sm" compact onClick={() => setCharting(true)}><ChartColumn aria-hidden /> Chart the selection</Button>
      </div>
      {notice && (
        <Alert tone="warning">
          {notice} <Button variant="ghost" size="sm" compact onClick={() => setNotice(null)}>Dismiss</Button>
        </Alert>
      )}
      <div ref={host} className="relative h-[70dvh] min-h-96 overflow-hidden rounded-md border border-border">
        <Grid model={model} mount={mount} />
        {hostBox && gridBox && shown.map((c) => {
          const at = parseCell(c.chart.spec.anchor ?? anchorFor(c.chart.spec.data));
          const p = at && cellPosition(model, sel.index, at.row, at.col);
          if (!p) return null;
          const left = gridBox.left - hostBox.left + p.x;
          const top = gridBox.top - hostBox.top + p.y;
          if (left > hostBox.width || top > gridBox.bottom - hostBox.top) return null;
          return (
            <div key={c.chart.id} className="absolute" style={{ left, top, ...CHART_SIZE }}>
              <SheetChart view={c} onEdit={(e) => edit([e])} />
            </div>
          );
        })}
      </div>
      {charting && <ChartDialog model={model} onClose={() => setCharting(false)} onAdd={(chart) => edit([{ op: 'add_chart', sheet: sel.sheet, chart }])} />}
    </div>
  );
}

/** A chart of the selection: its first column the labels, each other column a series, named by its first row. */
function ChartDialog({ model, onClose, onAdd }: { model: Model; onClose: () => void; onAdd: (c: ChartSpec) => void }) {
  const s = selectionOf(model);
  const [kind, setKind] = useState<ChartKind>('column');
  const [title, setTitle] = useState('');
  const header = s.r2 > s.r1;
  const ok = s.c2 > s.c1;
  const make = () => {
    const data = `${cellName(s.r1, s.c1)}:${cellName(s.r2, s.c2)}`;
    onAdd({ kind, data, ...(title.trim() ? { title: title.trim() } : {}), anchor: anchorFor(data) });
    onClose();
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title="Chart the selection" size="sm">
      {ok ? (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            {cellName(s.r1, s.c1)}:{cellName(s.r2, s.c2)} on {s.sheet}: the first column labels the chart, each other column is a series{header ? ', named by its first row' : ''}.
          </p>
          <Field label="Kind">
            <Select aria-label="Kind" options={KINDS} value={kind} onChange={(v) => setKind(v)} />
          </Field>
          <Field label="Title">
            <Input value={title} onChange={(e) => setTitle(e.target.value)} maxLength={200} />
          </Field>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose}>Cancel</Button>
            <Button onClick={make}>Add chart</Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="text-sm">Select the labels and at least one column of numbers (with their headers) first.</p>
          <div className="flex justify-end"><Button onClick={onClose}>OK</Button></div>
        </div>
      )}
    </Dialog>
  );
}
