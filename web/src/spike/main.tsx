// PDT-313 throwaway: IronCalc's own React spreadsheet over the daemon's live model, plus a PDT chart (Chart.js) drawn
// from sheet ranges. Run spikes/sheet's `serve` and `pnpm --dir web dev`, then open /sheet-spike.html.
import '@ironcalc/workbook/style.css';
import { IronCalc, Model, init } from '@ironcalc/workbook';
import { BarController, BarElement, CategoryScale, Chart, Legend, LinearScale, Title, Tooltip } from 'chart.js';
import { StrictMode, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

Chart.register(BarController, BarElement, CategoryScale, LinearScale, Legend, Title, Tooltip);

type ChartSpec = {
  id: string;
  kind: 'bar';
  title: string;
  sheet: string;
  categories: string;
  series: { name: string; values: string }[];
  anchor: string;
};

/** This view's id, so the daemon doesn't send it its own edits back. */
const VIEW = Math.random().toString(36).slice(2);

function cellRange(range: string): { r1: number; c1: number; r2: number; c2: number } {
  const cell = (s: string) => {
    const m = /^([A-Z]+)(\d+)$/.exec(s)!;
    return { r: Number(m[2]), c: [...m[1]].reduce((n, ch) => n * 26 + ch.charCodeAt(0) - 64, 0) };
  };
  const [a, b = a] = range.split(':');
  const [x, y] = [cell(a), cell(b)];
  return { r1: x.r, c1: x.c, r2: y.r, c2: y.c };
}

/** The cells of a one-column or one-row range. IronCalc's wasm (0.9.1) has no raw-value getter, so numbers are parsed
 *  back from the formatted value. */
function readRange(model: Model, sheet: number, range: string): string[] {
  const { r1, c1, r2, c2 } = cellRange(range);
  const out: string[] = [];
  for (let r = r1; r <= r2; r++) for (let c = c1; c <= c2; c++) out.push(model.getFormattedCellValue(sheet, r, c));
  return out;
}

function SheetChart({ model, spec, tick }: { model: Model; spec: ChartSpec; tick: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const chart = useRef<Chart | null>(null);
  useEffect(() => {
    const sheet = model.getWorksheetsProperties().findIndex((p) => p.name === spec.sheet);
    const labels = readRange(model, sheet, spec.categories);
    const datasets = spec.series.map((s, i) => ({
      label: s.name,
      data: readRange(model, sheet, s.values).map((v) => Number(v.replace(/[^0-9.-]/g, ''))),
      backgroundColor: ['#4e79a7', '#f28e2b', '#59a14f'][i % 3],
    }));
    if (!chart.current) {
      chart.current = new Chart(canvas.current!, {
        type: spec.kind,
        data: { labels, datasets },
        options: { animation: { duration: 150 }, plugins: { title: { display: true, text: spec.title } } },
      });
    } else {
      chart.current.data = { labels, datasets };
      chart.current.update();
    }
  }, [model, spec, tick]);
  useEffect(
    () => () => {
      chart.current?.destroy();
      chart.current = null;
    },
    [],
  );
  return <canvas ref={canvas} />;
}

function App() {
  const [model, setModel] = useState<Model | null>(null);
  const [charts, setCharts] = useState<ChartSpec[]>([]);
  const [tick, setTick] = useState(0); // any edit: redraw the charts
  const [remote, setRemote] = useState(0); // an edit from elsewhere: redraw the grid
  const [log, setLog] = useState<string[]>([]);
  const [agentAt, setAgentAt] = useState('Sales!B5');
  const [agentInput, setAgentInput] = useState('2600');

  useEffect(() => {
    let live = true;
    (async () => {
      await init();
      const bytes = new Uint8Array(await (await fetch('/spike/sheet')).arrayBuffer());
      const specs = (await (await fetch('/spike/charts')).json()) as ChartSpec[];
      if (live) {
        setModel(Model.fromBytes(bytes, 'en'));
        setCharts(specs);
      }
    })();
    return () => {
      live = false;
    };
  }, []);

  // The view's own edits go to the daemon as IronCalc diffs.
  useEffect(() => {
    if (!model) return;
    const timer = setInterval(() => {
      const diffs = model.flushSendQueue();
      if (diffs.length <= 1) return; // an empty queue
      fetch(`/spike/diffs?view=${VIEW}`, { method: 'POST', body: new Blob([diffs as Uint8Array<ArrayBuffer>]) });
      setTick((t) => t + 1);
    }, 200);
    return () => clearInterval(timer);
  }, [model]);

  // Everyone else's edits (an agent's) arrive as diffs on the event stream.
  useEffect(() => {
    if (!model) return;
    const events = new EventSource(`/spike/events?view=${VIEW}`);
    events.addEventListener('diffs', (e) => {
      const bytes = Uint8Array.from(atob((e as MessageEvent<string>).data), (c) => c.charCodeAt(0));
      model.applyExternalDiffs(bytes);
      // 0.9.1's widget has no refresh(); IronCalc's main branch adds `ref.workbook.refresh()`. Until then, remount it
      // (selection and scroll live in the model, so they survive).
      setTick((t) => t + 1);
      setRemote((t) => t + 1);
      setLog((l) => [`${new Date().toLocaleTimeString()} applied ${bytes.length} bytes of diffs`, ...l].slice(0, 5));
    });
    return () => events.close();
  }, [model]);

  const agentEdit = async () => {
    const res = await fetch('/spike/agent-edit', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify([{ at: agentAt, input: agentInput }]),
    });
    const text = res.ok ? ((await res.json()) as string[]).join(', ') : await res.text();
    setLog((l) => [`agent: ${text}`, ...l].slice(0, 5));
  };

  if (!model) return <p>Loading…</p>;
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 420px', height: '100vh', fontFamily: 'sans-serif' }}>
      <div style={{ position: 'relative', overflow: 'hidden' }}>
        <IronCalc key={remote} model={model} />
      </div>
      <aside style={{ padding: 12, borderLeft: '1px solid #ddd', overflow: 'auto', fontSize: 13 }}>
        {charts.map((c) => (
          <SheetChart key={c.id} model={model} spec={c} tick={tick} />
        ))}
        <h3>Agent edit (POST /spike/agent-edit)</h3>
        <input value={agentAt} onChange={(e) => setAgentAt(e.target.value)} style={{ width: 90 }} />{' '}
        <input value={agentInput} onChange={(e) => setAgentInput(e.target.value)} style={{ width: 160 }} />{' '}
        <button onClick={agentEdit}>Apply</button>
        <ul>
          {log.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      </aside>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
