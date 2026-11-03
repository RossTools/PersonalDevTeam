import { useState, type ReactNode } from 'react';
import { Copy, FolderGit2, GitBranch, PanelLeft, Pencil, Plus, Terminal, X } from 'lucide-react';
import { Button } from '@ui/Button';
import { ContextMenu, MenuButton, useMenuAt, type MenuEntry } from '@ui/Menu';
import { List, ListRow } from '@ui/List';
import { StatusDot } from '@ui/StatusDot';
import { Popover } from '@ui/Popover';
import { HoverCard } from '@ui/HoverCard';
import { Link } from '@ui/Link';
import { Badge } from '@ui/Badge';
import { Sheet } from '@ui/Sheet';
import { CommandList, type CommandItem } from '@ui/CommandList';
import { SectionLabel } from '@ui/SectionLabel';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}

function useLog() {
  const [last, setLast] = useState('—');
  return { last, log: (s: string) => () => setLast(s), set: setLast };
}

const tabEntries = (log: (s: string) => () => void, set: (s: string) => void): MenuEntry[] => [
  { label: 'Rename', icon: <Pencil aria-hidden />, shortcut: 'F2', onSelect: log('Rename') },
  { label: 'Move left', disabled: true, onSelect: log('Move left') },
  { label: 'Move right', onSelect: log('Move right') },
  { heading: 'Move to workspace' },
  { label: 'Release', onSelect: log('Moved to Release') },
  { label: 'Scratch', onSelect: log('Moved to Scratch') },
  { input: 'New workspace…', onSubmit: (v) => set(`New workspace "${v}"`) },
  { separator: true },
  {
    submenu: 'Copy',
    icon: <Copy aria-hidden />,
    entries: [
      { label: 'Branch name', onSelect: log('Copied branch') },
      { label: 'Session id', onSelect: log('Copied id') },
    ],
  },
  { label: 'Close tab', icon: <X aria-hidden />, danger: true, shortcut: 'Ctrl+] x', onSelect: log('Close tab') },
];

const places: CommandItem[] = [
  { id: 'new-claude', label: 'New Claude session', icon: <Terminal aria-hidden />, shortcut: 'Ctrl+] c' },
  { id: 'new-shell', label: 'New shell', icon: <Terminal aria-hidden />, shortcut: 'Ctrl+] s' },
  { id: 'pdt-main', label: 'main', group: 'PersonalDevTeam', icon: <GitBranch aria-hidden />, keywords: ['pdt'] },
  { id: 'pdt-ui', label: 'phase-3c-1-ui', group: 'PersonalDevTeam', icon: <GitBranch aria-hidden />, keywords: ['pdt'] },
  { id: 'demo', label: 'trunk', group: 'Demo app', icon: <FolderGit2 aria-hidden /> },
];

function MenusDemo() {
  const { last, log, set } = useLog();
  return (
    <Row label="Menu (right-click or long press the tab; ⋯ and a custom trigger)">
      <ContextMenu label="Tab Claude A" entries={() => tabEntries(log, set)}>
        <button type="button" className="h-control-md rounded-md border border-border bg-surface px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">
          Claude A (right-click me)
        </button>
      </ContextMenu>
      <MenuButton label="Tab actions" entries={() => tabEntries(log, set)} />
      <MenuButton label="Small actions" size="sm" entries={() => tabEntries(log, set)} />
      <MenuButton label="Run actions" placement="bottom start" entries={() => tabEntries(log, set)}>
        <Button variant="secondary">Run ▾</Button>
      </MenuButton>
      <span className="text-meta text-muted-foreground">Last: {last}</span>
    </Row>
  );
}

function MenuAtDemo() {
  const { last, log, set } = useLog();
  const { openMenu, menu } = useMenuAt();
  const agents = ['Claude A', 'Codex B', 'Shell'];
  return (
    <Row label="Menu at a point (ListRow onMenu: right-click, long press, Shift+F10)">
      <div className="w-64 rounded-md border border-border p-1">
        <List aria-label="Agents" onAction={(id) => set(`Opened ${id}`)}>
          {agents.map((a) => (
            <ListRow key={a} id={a} title={a} leading={<StatusDot status="idle" />}
              onMenu={(at) => openMenu(at, `Agent ${a}`, () => tabEntries(log, set))} />
          ))}
        </List>
      </div>
      {menu}
      <span className="text-meta text-muted-foreground">Last: {last}</span>
    </Row>
  );
}

function PopoverDemo() {
  const [picked, setPicked] = useState('—');
  return (
    <Row label="Popover">
      <Popover trigger={<Button variant="secondary">Filters</Button>} title="Filters">
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" defaultChecked /> Show ended sessions
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" /> Only this project
        </label>
      </Popover>
      <Popover trigger={<Button variant="secondary">Pick a place…</Button>} label="Places" className="w-80 p-1">
        {({ close }) => (
          <CommandList
            label="Places"
            placeholder="Project or branch…"
            items={places}
            onAction={(id) => {
              setPicked(id);
              close();
            }}
          />
        )}
      </Popover>
      <span className="text-meta text-muted-foreground">Picked: {picked}</span>
    </Row>
  );
}

function HoverCardDemo() {
  return (
    <Row label="HoverCard (hover, or focus with Tab)">
      <HoverCard
        content={
          <>
            <span className="font-semibold">PDT-42 Add the inbox</span>
            <span className="flex items-center gap-1.5"><Badge tone="accent">In progress</Badge> PR #57 · checks passing</span>
          </>
        }
      >
        <Link href="#PDT-42">PDT-42</Link>
      </HoverCard>
    </Row>
  );
}

function SheetDemo() {
  const [open, setOpen] = useState(false);
  return (
    <Row label="Sheet (swipe towards its edge to close)">
      <Sheet side="left" title="Workspaces" trigger={<Button variant="secondary"><PanelLeft aria-hidden /> Left</Button>}
        footer={<Button variant="ghost">Settings</Button>}>
        <ul className="flex flex-col gap-1 text-sm">
          <li>PersonalDevTeam</li>
          <li>Demo app</li>
          <li>Release</li>
        </ul>
      </Sheet>
      <Sheet side="right" title="PDT-42 Add the inbox" description="Ticket peek" trigger={<Button variant="secondary">Right</Button>}>
        <p className="text-sm text-muted-foreground">The ticket's summary, stage and latest activity.</p>
      </Sheet>
      <Button variant="secondary" onClick={() => setOpen(true)}>Bottom (controlled)</Button>
      <Sheet side="bottom" title="Actions" open={open} onOpenChange={setOpen}>
        <div className="flex flex-col gap-2 py-2">
          <Button variant="secondary" onClick={() => setOpen(false)}><Plus aria-hidden /> New tab</Button>
          <Button variant="danger" onClick={() => setOpen(false)}>Close tab</Button>
        </div>
      </Sheet>
    </Row>
  );
}

function CommandDemo() {
  const [chosen, setChosen] = useState('—');
  const [query, setQuery] = useState('');
  return (
    <Row label="CommandList (inline)">
      <div className="w-80 rounded-md border border-border bg-popover p-1">
        <CommandList label="Commands" items={places} onAction={setChosen} query={query} onQueryChange={setQuery} autoFocus={false} />
      </div>
      <span className="text-meta text-muted-foreground">Chosen: {chosen} · query “{query}”</span>
    </Row>
  );
}

export const section = {
  title: 'Menus and overlays',
  order: 30,
  render: () => (
    <>
      <MenusDemo />
      <MenuAtDemo />
      <PopoverDemo />
      <HoverCardDemo />
      <SheetDemo />
      <CommandDemo />
    </>
  ),
};
