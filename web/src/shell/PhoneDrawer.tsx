// The phone's drawer (PDT-112): the project, the views, the workspaces (folding open to their tabs), the agents,
// the theme switch, and Settings with the host's connection state. Opened from ☰ only.
import { useState } from 'react';
import { Plus, Settings } from 'lucide-react';
import { Button, IconButton, Select, Sheet, StatusDot, TreeView, type ThemePref, type TreeChildren, type TreeNode } from '@ui';
import { cn } from '@ui/lib/utils';
import type { Tab } from '../api/gen/Tab';
import { AgentDot, AgentLabel, itemState, UnseenDot } from '../common/AgentState';
import { InlineRename } from '../common/InlineRename';
import { runAction } from '../common/useAction';
import { itemOf } from '../workspaces/model';
import { AgentsSection, workspaceState } from '../workspaces/Sidebar';
import type { WorkspacesApi } from '../workspaces/useWorkspaces';
import { useConnected } from './connection';
import { useBackCloses } from './router';
import { ThemeSwitch } from './ThemeSwitch';
import { VIEWS, type ViewId } from './views';

const ALL = 'all';
const heading = 'flex items-center px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';
const row = 'h-11 w-full justify-start gap-3 px-2.5 font-normal';

type Node = TreeNode & { tab?: Tab };

export function PhoneDrawer({ open, onOpenChange, view, onView, counts, dots, ws, projectId, onProject, onRenameTab, theme, onTheme }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  view: ViewId;
  onView: (v: ViewId) => void;
  counts: Partial<Record<ViewId, number>>;
  /** A dot on a view with no count to show (the Inbox's unread Updates). */
  dots?: Partial<Record<ViewId, boolean>>;
  ws: WorkspacesApi;
  /** The project the phone's screens show (null: all), from the switcher. */
  projectId: string | null;
  onProject: (id: string | null) => void;
  onRenameTab: (tabId: string) => void;
  theme: ThemePref;
  onTheme: (pref: ThemePref) => void;
}) {
  const [adding, setAdding] = useState(false);
  const close = () => { setAdding(false); onOpenChange(false); };
  useBackCloses(open, close);
  const choose = (v: ViewId) => { onView(v); close(); };

  const node = (w: (typeof ws.workspaces)[number]): Node => {
    const state = workspaceState(ws, w);
    return {
      id: w.id,
      label: w.ticket ? `${w.ticket.key} ${w.ticket.title}` : w.name,
      hasChildren: w.tabs.length > 0,
      // Folded, a workspace shows its loudest state.
      icon: state ? <AgentDot state={state} unseenDot={false} /> : <span className="inline-block size-2" />,
      trailing: state && state.label !== 'Idle' ? <AgentLabel state={state} /> : undefined,
    };
  };
  const useChildren = (n: Node): TreeChildren<Node> => {
    const w = ws.workspaces.find((x) => x.id === n.id);
    return {
      status: 'ready',
      items: (w?.tabs ?? []).map((t) => {
        const info = ws.info(itemOf(t));
        return {
          id: `tab:${t.id}`,
          label: info?.title ?? 'Loading…',
          tab: t,
          icon: info ? <AgentDot state={itemState(info)} unseenDot={false} /> : <span className="inline-block size-2" />,
          trailing: info?.unseen ? <UnseenDot /> : undefined,
        };
      }),
    };
  };

  const connected = useConnected(ws.workspacesError);
  return (
    <Sheet side="left" title="Menu" hideTitle open={open} onOpenChange={(o) => (o ? onOpenChange(true) : close())} className="w-80 max-w-[85vw]">
      <div className="-mx-2 flex min-h-full flex-col">
        {/* Room on the right for the sheet's close button. */}
        <div className="pt-1 pr-10 pb-2 pl-2">
          <Select<string>
            aria-label="Project"
            options={[{ value: ALL, label: 'All projects' }, ...ws.projects.map((p) => ({ value: p.id, label: p.name }))]}
            value={projectId ?? ALL}
            onChange={(v) => onProject(v === ALL ? null : v)}
            className="w-full"
          />
        </div>
        <nav aria-label="Views" className="flex flex-col px-1">
          {VIEWS.filter((v) => v.id !== 'settings').map(({ id, label, icon: Icon, dot: dotName }) => {
            const n = counts[id] ?? 0;
            const dot = n === 0 && !!dots?.[id];
            return (
              <Button key={id} variant="ghost" aria-label={n > 0 ? `${label} (${n})` : dot ? `${label} (${dotName ?? 'new'})` : label} aria-current={view === id ? 'page' : undefined} onClick={() => choose(id)}
                className={cn(row, view === id ? 'bg-accent text-foreground' : 'text-muted-foreground')}>
                <Icon aria-hidden />
                <span className="min-w-0 flex-1 truncate text-left">{label}</span>
                {n > 0 && <span aria-hidden className="text-meta tabular-nums">{n}</span>}
                {dot && <span aria-hidden className="size-2 rounded-full bg-primary" />}
              </Button>
            );
          })}
        </nav>
        <section aria-label="Workspaces" className="px-1">
          <h2 className={heading}>
            <span className="flex-1">Workspaces</span>
            <IconButton label="New workspace" size="sm" noTooltip className="size-5" onClick={() => setAdding(true)}>
              <Plus aria-hidden />
            </IconButton>
          </h2>
          {adding && (
            <div className="px-1 pb-1">
              <InlineRename
                label="New workspace name"
                placeholder="Workspace name"
                // Creating one opens it, so the drawer closes.
                onCommit={(name) => { close(); void runAction(() => ws.createWorkspace(name)); }}
                onCancel={() => setAdding(false)}
              />
            </div>
          )}
          <TreeView<Node>
            aria-label="Workspaces"
            items={ws.workspaces.map(node)}
            useChildren={useChildren}
            defaultExpandedKeys={ws.workspace ? [ws.workspace.id] : []}
            selectedKey={ws.tab ? `tab:${ws.tab.id}` : null}
            onAction={(n) => {
              if (n.tab) ws.selectTab(n.tab);
              else ws.selectWorkspace(n.id);
              close();
            }}
          />
        </section>
        <div className="px-1">
          <AgentsSection ws={ws} onRenameTab={onRenameTab} onChoose={close} />
        </div>
        <div className="mt-auto border-t border-border px-2 pt-2">
          <ThemeSwitch value={theme} onChange={onTheme} layout="labels" />
        </div>
        <div className="flex items-center gap-2 px-1 pt-1">
          <Button variant="ghost" aria-current={view === 'settings' ? 'page' : undefined} onClick={() => choose('settings')}
            className={cn(row, 'flex-1', view === 'settings' ? 'bg-accent text-foreground' : 'text-muted-foreground')}>
            <Settings aria-hidden />
            <span>Settings</span>
          </Button>
          <span className="flex shrink-0 items-center gap-1.5 px-2 text-meta text-muted-foreground">
            <StatusDot status={connected ? 'running' : 'failed'} label={connected ? 'Host connected' : "Can't reach the host"} />
            {connected ? 'Host connected' : "Can't reach the host"}
          </span>
        </div>
      </div>
    </Sheet>
  );
}
