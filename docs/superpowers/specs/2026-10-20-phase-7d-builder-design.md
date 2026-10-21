# Personal Dev Team — Phase 7d: the workflow builder

Status: design for PDT-79, 2026-10-20. Builds §5 and §6 of the vision
(docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md) on 7a's document format, engine and API
(docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md, docs/reference/workflows.md). Agent steps and the Inbox
are 7b (PDT-77): the builder draws every node from the catalog, so 7b's agent step needs no builder change.

## 1. What 7d delivers

- A **Workflows view** in the rail: the list of workflows and templates, and each workflow's builder, runs and
  versions. Settings → This host → Workflows (7a) goes; this replaces it.
- Three writable **builder views** of one document: **Graph**, **Outline** and **Text** (JSON or TOML). They edit one
  draft, so a change in one shows in the others; Save makes a version.
- A **Runs view**: every run with its trigger, status and steps; the graph shows the path a run took; retry from the
  failed step, cancel, and answer a waiting gate.
- **Templates**: built-in and the user's own, with parameters; instantiating one makes an ordinary workflow.
- **Versions**: history with diffs, restore; **proposals**: a version an agent writes arrives proposed, and the user
  reviews its diff and accepts or discards it.
- **Validate and dry-run** in the builder, before enabling.

Done when the user can create a workflow from a template, edit it in either the graph or the outline view, dry-run it,
enable it and watch its runs.

## 2. Layout

- **Rail**: Workflows (lucide `Workflow`), between Tickets and Files; on phones it is the fifth item.
- **List column**: *New workflow* (opens the template picker, §5), then *Workflows* grouped by where they apply
  (This host, Personal, each project by name), each row with an on/off dot and its badges (Built in, In the repo,
  Proposed); then *Templates* (built-in and the user's).
- **Work area** for a workflow: a `ViewHeader` (name, badges, the validation state, the **Enabled** switch, **Dry
  run**, **Run now** when it has a manual trigger, and a ⋯ menu: Save as template, Duplicate, Delete…), then a
  `SegmentedControl`: **Graph | Outline | Text | Runs | Versions**. Graph is the default on desktop, Outline on a phone.
- **Draft bar** (under the header) while the draft differs from the saved version: "Unsaved changes", a note field,
  Discard and Save. Save is refused (with the errors on the nodes) while the draft has errors. Polling never replaces
  a dirty draft; a version saved elsewhere meanwhile shows "Saved elsewhere: reload" in the bar.
- **Read-only** workflows (from the repo) show the same views without editing and say where the file is.
- **Proposal banner** when an agent proposed a version: "An agent proposed version N · Review" → Versions.

## 3. The builder

All three views edit one `WorkflowDoc` draft held by the workflow page. Every change is checked by the server
(`POST /api/workflows/check`, debounced): the issues, and each node's ports as its config gives them (§6.1), so dynamic
ports (a gate's choices, a switch's cases, 7b's outcomes) come from the kind, not from the builder.

### 3.1 Graph view

React Flow (`@xyflow/react`), wrapped in web/ui as `FlowGraph`: features never import xyflow (the lint test checks).

- A node is a card: kind icon by category, title (or id) and kind, its issues (a red/amber marker with the messages),
  input handle on the left (none for triggers), one output handle per port on the right with the port's title, and
  the `error` port when it has an edge (offered in the panel otherwise).
- **Add**: a *Add node* button opens the catalog as a `CommandList` (searchable, grouped by category); the node lands
  right of the selected node (and is wired from its first open port) or at the free spot.
- **Connect**: drag from a port to a node. Dropping on empty space is ignored. One edge per (from, port, to).
- **Move**: drag; positions are saved in the document (`position`). Nodes without one get an automatic layered layout
  (depth from the triggers).
- **Select** a node: the side panel (§3.4). Delete/Backspace removes the selected node or edge.
- In a run or dry run (§4), nodes show their step status and the taken edges are highlighted.

### 3.2 Outline view

The same document as ordered steps, for linear workflows, keyboard and phone.

- Each trigger is a top-level row; below it, what its port leads to. A node with one port lists its successors in
  order; a node with several ports nests a branch per port ("If true", "If false", "finish", "Not asked").
- A node reached a second time (two triggers into one action, a loop) shows once in full and elsewhere as
  "→ continues at *tell*".
- Rows are a tree (`TreeView`: arrow keys move, Enter opens the node's panel). Each row has a ⋯ menu: Add step after,
  Edit, Move up / Move down (in a chain: swaps with its neighbour), Remove. Each branch has *Add step*. Adding after a
  node inserts it between the node and its successors; removing a node joins its predecessor to its successors when it
  had one way in and one way out.
- On a phone the panel opens in a sheet.

### 3.3 Text view

The draft as **JSON** or **TOML** (a `SegmentedControl`), in a CodeMirror editor. Edits are parsed as you type; a parse
error shows under the editor and leaves the draft unchanged; a parsed document becomes the draft (and is checked like
any other change). TOML has no null: null values are left out, which the document format reads the same.

### 3.4 Node panel

Opened by selecting a node in the graph or outline. Generated from the kind's `configSchema` (JSON Schema from
schemars):

- id (renaming rewrites its edges and `{{steps.<id>}}` references), title, the kind's description;
- the config: strings, numbers, booleans, string enums, lists of strings, lists of objects (conditions, cases, choices
  ahead), optional fields and `Tpl` fields (a literal or a reference); anything the form can't draw falls back to a
  JSON field for that property;
- a reference picker on text fields: `{{trigger.*}}` paths from the workflow's trigger kinds' output schemas and
  `{{steps.<id>}}` for the nodes before it;
- settings every node has: retry (`max`, `backoffMs`), `timeoutMs`, `maxVisits`, and "Handle errors" (adds the
  `error` port);
- the node's issues; Remove node.

## 4. Runs, dry runs and gates

- **Runs** tab: the workflow's runs, newest first (status, trigger type, start, duration, ticket key). Selecting one
  shows its graph with the path (step status per node, taken edges highlighted), the steps in order (status, port,
  attempts, error, input and output as JSON), and its actions: **Retry from the failed step** (failed runs), **Cancel**
  (running or waiting), and for a waiting gate its question with a button per choice (`/steps/{step}/answer`). The
  list of everything waiting on the user is 7b's Inbox.
- **Dry run** (header button): choose the event: a recent event from the log of a type the workflow triggers on, or a
  hand-written one (type and payload JSON, prefilled with the trigger's fields), or the manual input. It runs the
  **draft**, so it can be tried before saving. The result shows like a run (graph path and steps, triggers that did
  not match and why, actions as "would …"), without changing anything.
- **Enable**: the switch asks for confirmation when turning a workflow on ("Enable *name*? It runs on: … and can: …",
  listing its triggers and actions; enabling is consent, vision principle 5), offering **Dry run first**. Enabling an
  invalid workflow is refused (7a).

## 5. Templates

- A **template** is a document with parameters: `{id, name, description, params, doc}`. A param is `{id, title,
  description, type: string | text | number | boolean | choice, choices, default, required}`. The document uses
  `{{params.<id>}}`; instantiating fills them in: a string that is exactly one reference takes the value with its type
  (a boolean stays a boolean), others are interpolated as text. Other references (`{{trigger.x}}`) are left alone.
- **Built-in templates** ship in pdtd (`builtin:<id>`, read-only): the four built-in workflows (so they can be copied,
  e.g. per project), plus *Tell the agent when CI fails* (message prefix), *Ask before finishing a merged ticket*
  (delete branch?), *Comment when a pull request merges* (comment text) and *Blank* (a Run now trigger).
- **The user's templates** live in pdtd (`workflow_templates`). *Save as template* on a workflow copies its document;
  the template page edits the name, description, params (a small table) and the document as text, and shows the
  document's issues with the params' defaults filled in.
- **New workflow** dialog: pick a template, fill its params (a form from the param list), name the workflow and choose
  where it applies (this host, a project, personal). The new workflow starts **disabled**, so the user tries it before
  enabling.

## 6. API additions

### 6.1 Check

`POST /api/workflows/check {doc}` → `{issues, ports: {<node>: [{id, title}]}}`: validation (7a) plus each node's ports
from its kind and config (`error` excluded). The builder calls it on every change.

### 6.2 Versions and proposals

- `workflow_versions` gains `status`: `saved` (the normal case), `proposed`, `discarded`. A version's number is the
  highest so far plus one, so a proposal and later saves never collide.
- `PUT /api/workflows/{id} {doc, note, propose: true}` saves a **proposal** (author `agent`): validated like a save,
  but the current version doesn't change. A newer proposal discards the older pending one. Repo workflows refuse
  (409 `workflow_in_repo`).
- `POST /api/workflows/{id}/versions/{v}/accept` makes a proposed version current (validated again);
  `POST /api/workflows/{id}/versions/{v}/discard` marks it discarded. Both 409 `not_proposed` otherwise.
- `Workflow` gains `proposed` (the pending proposal's number, or null); `WorkflowVersion` gains `status`.
- **Restore** is a save of an older version's document (note "Restored vN"); no route of its own.
- Agents are told (agent guide) to propose rather than save, so the user reviews their change.

### 6.3 Templates

- `GET /api/workflow-templates` (built-in first, then the user's by name), `GET /api/workflow-templates/{id}`,
  `POST` `{name, description, params, doc}`, `PUT /{id}`, `DELETE /{id}` (built-ins: 409 `builtin_template`).
- `POST /api/workflow-templates/{id}/instantiate {params, name, scope, projectId}` → the new workflow (disabled;
  `422 workflow_invalid` when the filled-in document doesn't validate; `400 invalid_input` for a missing required or
  ill-typed param).
- A template's document is checked on save with its params' defaults filled in (placeholder values for params without
  one); errors refuse the save.

## 7. Code layout

- pdt-core `workflow/template.rs` (params, filling in, checks), `api.rs` (`VersionStatus`, `proposed`, template
  records).
- pdt-store migration `0022_workflow_builder.sql` (version status, `workflow_templates`), `workflows.rs` (proposals,
  templates).
- pdt-api `workflow/templates.rs` (built-in templates, routes), `routes.rs` (check, propose, accept, discard).
- web: `web/ui/FlowGraph.tsx` (the xyflow wrapper, with its gallery section); `web/src/workflows/` — `WorkflowsList`,
  `WorkflowPage`, `useDraft`, `model.ts` (document edits, layout, outline tree), `GraphView`, `OutlineView`,
  `TextView`, `NodePanel` + `SchemaForm`, `RunsView`, `DryRunDialog`, `VersionsView`, `TemplatePage`,
  `NewWorkflowDialog`, `EnableSwitch`.
- docs: docs/reference/workflows.md (builder, templates, proposals), docs/agent-guide.md (propose), AGENTS.md layout
  line if needed.
