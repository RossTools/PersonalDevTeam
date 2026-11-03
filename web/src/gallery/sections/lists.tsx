import { useState, type ReactNode } from 'react';
import { File, Folder } from 'lucide-react';
import { Badge } from '@ui/Badge';
import { Button } from '@ui/Button';
import { StatusDot, type Status } from '@ui/StatusDot';
import { Input } from '@ui/Input';
import { Switch } from '@ui/Switch';
import { MenuButton } from '@ui/Menu';
import { List, ListRow, type ListDropEvent } from '@ui/List';
import { TreeView, type TreeChildren, type TreeNode } from '@ui/TreeView';
import { ViewHeader } from '@ui/ViewHeader';
import { Page, Section, SettingsRow, SettingsScreen } from '@ui/Page';
import { SectionLabel } from '@ui/SectionLabel';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      {children}
    </div>
  );
}

// --- Sidebar-like lists -------------------------------------------------------------------------------------------

const AGENTS: { id: string; title: string; where: string; status: Status; state: string; unseen?: boolean }[] = [
  { id: 's1', title: 'fix login redirect', where: 'PDT · fix-login', status: 'running', state: 'Working' },
  { id: 's2', title: 'tickets schema', where: 'PDT · tickets', status: 'waiting', state: 'Needs you' },
  { id: 's3', title: 'review copy', where: 'Site · main', status: 'idle', state: 'Done', unseen: true },
  { id: 's4', title: 'old experiment', where: 'no tab · scratch', status: 'ended', state: 'Ended' },
];

const tone = (state: string) =>
  state === 'Needs you' ? 'font-semibold text-waiting' : state === 'Working' ? 'text-running' : 'text-muted-foreground';

function Agents() {
  const [current, setCurrent] = useState('s2');
  const [menuAt, setMenuAt] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-1">
      <List aria-label="Agents" items={AGENTS} onAction={setCurrent} className="w-72 rounded-md border border-border bg-background p-1">
        {(a) => (
          <ListRow
            id={a.id}
            title={a.title}
            secondary={a.where}
            current={a.id === current}
            bold={a.unseen}
            leading={<StatusDot status={a.status} pulse={a.status === 'running'} />}
            trailing={
              <>
                <span className={`text-meta ${tone(a.state)}`}>{a.state}</span>
                {a.unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-primary" />}
              </>
            }
            onMenu={() => setMenuAt(a.title)}
          />
        )}
      </List>
      <p className="text-meta text-muted-foreground">
        Right-click, long press or Shift+F10 on a row: {menuAt ? `menu for “${menuAt}”` : 'no menu yet'}.
      </p>
    </div>
  );
}

function Workspaces() {
  const [items, setItems] = useState([
    { id: 'w1', name: 'PDT', key: 'PDT', tabs: 4 },
    { id: 'w2', name: 'Site', key: 'WEB', tabs: 2 },
    { id: 'w3', name: 'Scratch', key: null, tabs: 0 },
  ]);
  const onDrop = ({ id, target }: ListDropEvent) => {
    if (!target || target.id === id) return;
    setItems((list) => {
      const moving = list.find((w) => w.id === id)!;
      const rest = list.filter((w) => w.id !== id);
      const at = rest.findIndex((w) => w.id === target.id) + (target.position === 'after' ? 1 : 0);
      return [...rest.slice(0, at), moving, ...rest.slice(at)];
    });
  };
  return (
    <List aria-label="Workspaces" items={items} dragKind="workspace" onDrop={onDrop} onAction={() => {}}
      className="w-72 rounded-md border border-border bg-background p-1">
      {(w) => (
        <ListRow
          id={w.id}
          title={w.name}
          current={w.id === 'w1'}
          trailing={
            <>
              {w.key && <span className="font-mono text-2xs text-muted-foreground">{w.key}</span>}
              <span className="text-meta text-muted-foreground tabular-nums">{w.tabs}</span>
            </>
          }
        />
      )}
    </List>
  );
}

// --- File tree ----------------------------------------------------------------------------------------------------

interface FileNode extends TreeNode {
  status?: string;
}

const FILES: Record<string, FileNode[]> = {
  '': [
    { id: 'crates', label: 'crates', hasChildren: true },
    { id: 'web', label: 'web', hasChildren: true },
    { id: 'empty', label: 'empty', hasChildren: true },
    { id: 'README.md', label: 'README.md', status: 'M' },
    { id: 'old.txt', label: 'old.txt', status: 'D' },
  ],
  crates: [{ id: 'crates/pdtd', label: 'pdtd', hasChildren: true }],
  'crates/pdtd': [{ id: 'crates/pdtd/main.rs', label: 'main.rs' }],
  web: [
    { id: 'web/ui', label: 'ui', hasChildren: true },
    { id: 'web/package.json', label: 'package.json', status: '??' },
  ],
  'web/ui': [{ id: 'web/ui/List.tsx', label: 'List.tsx', status: 'A' }],
  empty: [],
};

const STATUS_TONE: Record<string, string> = { M: 'text-waiting', A: 'text-running', '??': 'text-running', D: 'text-failed' };

const decorate = (n: FileNode): FileNode => ({
  ...n,
  title: n.id,
  icon: n.hasChildren ? <Folder size={13} aria-hidden /> : <File size={13} aria-hidden />,
  labelClassName: n.status === 'D' ? 'line-through text-muted-foreground' : undefined,
  trailing: n.status ? (
    <span className={`font-mono text-2xs ${STATUS_TONE[n.status]}`}>{n.status === '??' ? 'U' : n.status}</span>
  ) : undefined,
});

/** Stands in for FolderView's one useQuery per folder: web/ui fails, the rest resolve at once. */
function useFolder(node: FileNode): TreeChildren<FileNode> {
  if (node.id === 'web/ui') return { status: 'error', message: 'Permission denied' };
  return { status: 'ready', items: (FILES[node.id] ?? []).map(decorate) };
}

function Files() {
  const [file, setFile] = useState<string | null>('README.md');
  return (
    <div className="flex flex-col gap-1">
      <TreeView aria-label="Files" items={FILES[''].map(decorate)} useChildren={useFolder} defaultExpandedKeys={['crates']}
        selectedKey={file} onAction={(n) => setFile(n.id)} className="w-72 rounded-md border border-border bg-background p-1" />
      <p className="text-meta text-muted-foreground">Open: {file ?? 'nothing'} (web/ui fails to load, on purpose)</p>
      <TreeView aria-label="Loading folder" items={[{ id: 'slow', label: 'slow folder', hasChildren: true, icon: <Folder size={13} aria-hidden /> }]}
        useChildren={() => ({ status: 'loading' })} defaultExpandedKeys={['slow']} className="w-72 rounded-md border border-border bg-background p-1" />
    </div>
  );
}

// --- Headers and pages --------------------------------------------------------------------------------------------

function Headers() {
  const more = () => [
    { label: 'Rename', onSelect: () => {} },
    { separator: true as const },
    { label: 'Remove', onSelect: () => {}, danger: true },
  ];
  return (
    <div className="flex flex-col gap-3">
      <div className="rounded-md border border-border bg-background">
        <ViewHeader
          leading={<StatusDot status="running" pulse />}
          title="fix login redirect"
          titleTooltip="fix login redirect (claude)"
          meta={<><span className="text-running">Working</span><span className="truncate">~/work/PersonalDevTeam/.worktrees/fix-login</span></>}
          actions={<><Button variant="ghost" size="sm">Compose</Button><Button variant="ghost" size="sm">Stop</Button><MenuButton label="More actions" size="sm" entries={more} /></>}
        />
        <div className="p-3 text-meta text-muted-foreground">terminal…</div>
      </div>
      <div className="w-[360px] max-w-full rounded-md border border-border bg-background">
        <ViewHeader
          leading={<StatusDot status="failed" />}
          title="pdt · phase-3c-1-ui"
          meta={<span>Failed (exit 1) · http://127.0.0.1:8512</span>}
          actions={<><Button variant="primary" size="sm">Restart</Button><MenuButton label="More actions" size="sm" entries={more} /></>}
        />
        <div className="p-3 text-meta text-muted-foreground">phone width: the actions wrap to a second line</div>
      </div>
    </div>
  );
}

function SettingsSample() {
  const [remote, setRemote] = useState(true);
  return (
    <div className="flex h-[36rem] flex-col rounded-md border border-border bg-background">
      <Page title="This host" description="Settings for the machine running pdtd.">
        <Section
          label="Updates"
          description="New releases, from GitHub."
          actions={<Button size="sm">Check for updates</Button>}
        >
          <SettingsRow label="Version 0.1.6" description="Last checked 10:42.">
            <Badge tone="accent">v0.1.7 available</Badge>
          </SettingsRow>
          <SettingsRow label="Serve over Tailscale" description="https://mac-mini.tailnet.ts.net">
            <Switch aria-label="Serve over Tailscale" checked={remote} onChange={setRemote} />
          </SettingsRow>
        </Section>
        <Section label="Runs" footer={<Button size="sm" variant="primary">Save</Button>}>
          <SettingsRow label="Ports" description="Runs get a port from this range.">
            <Input aria-label="First run port" defaultValue="8400" className="w-20" />
            <span className="text-muted-foreground">–</span>
            <Input aria-label="Last run port" defaultValue="8499" className="w-20" />
          </SettingsRow>
          <SettingsRow stacked label="Host overrides" description="Stacked: label above a full-width control.">
            <Input aria-label="Host overrides" defaultValue="[[run]]" />
          </SettingsRow>
        </Section>
        <Section label="Danger zone" danger>
          <SettingsRow label="Remove this project" description="Stops its sessions and runs. Files stay on disk.">
            <Button variant="danger" size="sm">Remove project…</Button>
          </SettingsRow>
        </Section>
      </Page>
    </div>
  );
}

function SettingsScreenSample() {
  return (
    <div className="flex h-72 flex-col rounded-md border border-border bg-background">
      <SettingsScreen crumb="This host" lead="Storage">
        <Section label="Storage" description="What PDT keeps on this machine, and what cleaning frees.">
          <SettingsRow label="Clean up automatically" description="Ended sessions' logs older than 30 days.">
            <Switch aria-label="Clean up automatically" checked onChange={() => {}} />
          </SettingsRow>
        </Section>
      </SettingsScreen>
    </div>
  );
}

export const section = {
  title: 'Lists and layout',
  order: 40,
  render: () => (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap gap-6">
        <Demo label="List · agents (current, unseen, two lines, row menu)"><Agents /></Demo>
        <Demo label="List · workspaces (drag the handle to reorder)"><Workspaces /></Demo>
        <Demo label="TreeView · files (lazy folders, git status)"><Files /></Demo>
      </div>
      <Demo label="ViewHeader"><Headers /></Demo>
      <Demo label="Page · Section · SettingsRow"><SettingsSample /></Demo>
      <Demo label="SettingsScreen · one section on a screen of its own (the section's heading is the title)"><SettingsScreenSample /></Demo>
    </div>
  ),
};
