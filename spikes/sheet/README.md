# PDT-313 sheet prototype (throwaway, never merged)

IronCalc on both sides: the daemon's model in Rust (`ironcalc` 0.9.1), the browser's in WASM (`@ironcalc/workbook`
0.9.1, the same version: the two exchange IronCalc's bitcode). The file of record is `.xlsx`; charts are PDT's own spec
beside it (`<name>.charts.json`), drawn with Chart.js.

```sh
cd spikes/sheet && cargo build
B=target/debug/pdt-sheet-spike
$B seed /tmp/sales.xlsx                       # values, formulas, formatting, two sheets, a chart spec
$B read /tmp/sales.xlsx                       # inputs, formulas and computed values, no browser
$B set /tmp/sales.xlsx 'Sales!B2' 3000        # edit, recalculate, save, no browser
$B serve /tmp/sales.xlsx 7499                 # the live model a view and agents share
pnpm --dir web dev                            # then open /sheet-spike.html (proxies /spike to 7499)
curl -s localhost:7499/spike/agent-edit -H 'content-type: application/json' \
  -d '[{"at":"Sales!B5","input":"4000"}]'     # an agent's edit: shows in the open view
```

Endpoints (`serve`): `GET /spike/sheet` (IronCalc bytes), `GET /spike/charts`, `GET /spike/events?view=<id>` (SSE,
base64 IronCalc diffs from everyone but `<id>`), `POST /spike/diffs?view=<id>` (a view's `flushSendQueue()`),
`POST /spike/agent-edit` (`[{at, input}]`). Every edit saves the `.xlsx` (written to a temp file, then renamed).
