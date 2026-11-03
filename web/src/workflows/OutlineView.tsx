// The outline view: the document as ordered steps with a branch per port. Up/Down move between
// rows, Enter opens a step's panel, each step's ⋯ menu adds, moves and removes.
import { useRef, type KeyboardEvent, type ReactNode } from 'react';
import { CornerDownRight, Plus } from 'lucide-react';
import { Button, MenuButton, Popover, SectionLabel, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import type { KindSpec } from '../api/gen/KindSpec';
import type { WorkflowCheck } from '../api/gen/WorkflowCheck';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { KindIcon, KindList } from './KindPicker';
import type { PathOverlay } from './GraphView';
import { fieldLabel, runForm } from './runInputs';
import {
  addNode, canMove, issuesOf, kindOf, moveInChain, newNode, nodeTitle, onlyPort, outline, portsOf, removeNode,
  spotAfter, type Catalog, type OutlineBranch, type OutlineItem,
} from './model';

export interface OutlineViewProps {
  doc: WorkflowDoc;
  onChange?: (doc: WorkflowDoc) => void;
  catalog: Catalog;
  check?: WorkflowCheck | null;
  selected: string | null;
  onSelect: (id: string | null) => void;
  readOnly?: boolean;
  path?: PathOverlay;
}

const row = 'flex min-h-control-md w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch';

const STATUS_DOT: Record<string, string> = {
  running: 'bg-running', waiting: 'bg-waiting', failed: 'bg-failed', succeeded: 'bg-running', skipped: 'bg-idle',
};

/** "Add step" in a branch or after a step: the catalog in a popover. */
function AddStep({ catalog, onPick, label = 'Add step', triggers = false }: { catalog: Catalog; onPick: (k: KindSpec) => void; label?: string; triggers?: boolean }) {
  return (
    <Popover
      label={label}
      className="w-80 p-0"
      trigger={<Button size="sm" variant="ghost" data-outline-row className="text-muted-foreground"><Plus aria-hidden /> {label}</Button>}
    >
      {({ close }) => <KindList catalog={catalog} triggers={triggers} onPick={(k) => { close(); onPick(k); }} />}
    </Popover>
  );
}

export function OutlineView({ doc, onChange, catalog, check, selected, onSelect, readOnly = false, path }: OutlineViewProps) {
  const edit = readOnly || !onChange ? undefined : onChange;
  const tree = outline(doc, catalog, check);
  const host = useRef<HTMLDivElement>(null);
  const firstPort = (id: string) => portsOf(doc.nodes.find((n) => n.id === id)!, catalog, check)[0]?.id ?? 'out';

  const insert = (kind: KindSpec, after?: { node: string; port: string }) => {
    if (!edit) return;
    const node = newNode(doc, kind, spotAfter(doc, catalog, after?.node));
    // A kind whose ports come from its config has none to continue by yet: it joins the branch instead.
    const through = kind.dynamicPorts ? undefined : kind.ports[0]?.id;
    edit(after && through ? addNode(doc, node, after, through) : after ? { ...addNode(doc, node), edges: [...doc.edges, { from: after.node, port: after.port, to: node.id }] } : addNode(doc, node));
    onSelect(node.id);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const rows = [...(host.current?.querySelectorAll<HTMLElement>('[data-outline-row]') ?? [])];
    const i = rows.indexOf(document.activeElement as HTMLElement);
    const next = rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next) {
      e.preventDefault();
      next.focus();
    }
  };

  const menu = (id: string): MenuEntry[] => {
    const node = doc.nodes.find((n) => n.id === id)!;
    const port = onlyPort(doc, node, catalog, check);
    const entries: MenuEntry[] = [{ label: 'Edit', onSelect: () => onSelect(id) }];
    if (!edit) return entries;
    if (port) {
      entries.push({
        submenu: 'Add step after',
        entries: catalog.filter((k) => k.category !== 'trigger').map((k) => ({ label: k.title, onSelect: () => insert(k, { node: id, port }) })),
      });
    }
    entries.push(
      { label: 'Move up', disabled: !canMove(doc, catalog, id, 'up'), onSelect: () => edit(moveInChain(doc, catalog, id, 'up', firstPort)) },
      { label: 'Move down', disabled: !canMove(doc, catalog, id, 'down'), onSelect: () => edit(moveInChain(doc, catalog, id, 'down', firstPort)) },
      { separator: true },
      { label: 'Remove', danger: true, onSelect: () => { edit(removeNode(doc, id)); if (selected === id) onSelect(null); } },
    );
    return entries;
  };

  const item = (it: OutlineItem): ReactNode => {
    const node = doc.nodes.find((n) => n.id === it.id);
    if (!node) return null;
    const title = nodeTitle(node, catalog);
    if (it.type === 'ref') {
      return (
        <li key={it.path}>
          <button type="button" data-outline-row className={cn(row, 'text-muted-foreground')} onClick={() => onSelect(it.id)}>
            <CornerDownRight aria-hidden className="size-4" /> Continues at {title}
          </button>
        </li>
      );
    }
    const issues = issuesOf(check, it.id);
    const status = path?.status.get(it.id);
    // Run now's declared inputs (PDT-301).
    const inputs = node.kind === 'manual' ? runForm({ ...doc, nodes: [node] })?.fields ?? [] : [];
    const asks = inputs.map(fieldLabel).join(', ');
    return (
      <li key={it.path}>
        <div className={cn('flex items-center gap-1 rounded-md', selected === it.id && 'bg-accent')}>
          <button type="button" data-outline-row aria-current={selected === it.id ? 'true' : undefined} className={cn(row, 'flex-1')} onClick={() => onSelect(it.id)}>
            <span className="flex shrink-0 text-muted-foreground [&_svg]:size-4"><KindIcon category={kindOf(catalog, node.kind)?.category} /></span>
            <span className="min-w-0 truncate font-medium">{title}</span>
            <span className="min-w-0 truncate font-mono text-meta text-muted-foreground">{it.id}</span>
            {status && <span role="img" aria-label={status} className={cn('size-2 shrink-0 rounded-full', STATUS_DOT[status])} />}
            {issues.length > 0 && (
              <span role="img" aria-label={issues.map((i) => i.message).join('; ')} title={issues.map((i) => i.message).join('\n')}
                className={cn('ml-auto size-2 shrink-0 rounded-full', issues.some((i) => i.severity === 'error') ? 'bg-failed' : 'bg-waiting')} />
            )}
          </button>
          {/* Read-only, its one entry (Edit) is the row's own tap. */}
          {edit && <MenuButton label={`Actions for ${title}`} size="sm" entries={() => menu(it.id)} />}
        </div>
        {asks && <p className="truncate px-2 pl-8 text-meta text-muted-foreground">Asks for {asks}</p>}
        {it.branches.length > 0 && <ul className="ml-4 border-l border-border pl-2">{it.branches.map(branch)}</ul>}
      </li>
    );
  };

  const branch = (b: OutlineBranch): ReactNode => (
    <li key={`${b.from}:${b.port}`} className="py-0.5">
      <SectionLabel as="div" className="flex items-center gap-2 px-2 pt-1">{b.label}</SectionLabel>
      <ul className="flex flex-col">{b.items.map(item)}</ul>
      {edit && <AddStep catalog={catalog} onPick={(k) => insert(k, { node: b.from, port: b.port })} />}
    </li>
  );

  return (
    <div ref={host} onKeyDown={onKeyDown} className="min-h-0 flex-1 overflow-auto p-3">
      <ul aria-label="Steps" className="flex flex-col gap-1">{tree.roots.map(item)}</ul>
      {edit && <AddStep catalog={catalog} triggers label="Add trigger" onPick={(k) => insert(k)} />}
      {tree.loose.length > 0 && (
        <section aria-label="Not connected" className="mt-4">
          <SectionLabel className="px-2 pb-1">Not connected</SectionLabel>
          <ul className="flex flex-col gap-1">{tree.loose.map(item)}</ul>
        </section>
      )}
      {doc.nodes.length === 0 && <p className="px-2 text-sm text-muted-foreground">No steps yet. Add a trigger to start.</p>}
    </div>
  );
}
