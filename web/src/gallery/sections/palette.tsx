import { useState } from 'react';
import { Button } from '@ui/Button';
import { Kbd } from '@ui/Kbd';
import { CommandPalette } from '../../palette/CommandPalette';
import type { PaletteItem } from '../../palette/paletteItems';

function items(log: (s: string) => () => void): PaletteItem[] {
  const go = (id: string, label: string, extra: Partial<PaletteItem> = {}): PaletteItem =>
    ({ id, label, group: 'Go to', run: log(label), ...extra });
  const act = (id: string, label: string, extra: Partial<PaletteItem> = {}): PaletteItem =>
    ({ id, label, group: 'Actions', run: log(label), ...extra });
  return [
    go('w1', 'Workspace · PersonalDevTeam', { icon: 'workspace' }),
    go('w2', 'Workspace · Scratch', { icon: 'workspace' }),
    go('t1', 'fix login redirect', { detail: 'PersonalDevTeam', keywords: ['PDT', 'fix-login'], state: { dot: 'running', pulse: true, label: 'Working', tone: 'running', unseen: false } }),
    go('t2', 'tickets schema', { detail: 'PersonalDevTeam', keywords: ['PDT', 'tickets'], state: { dot: 'waiting', pulse: false, label: 'Needs you', tone: 'waiting', unseen: true } }),
    go('t3', 'pdt · phase-3c-2-shell', { detail: 'Scratch', keywords: ['PDT'], state: { dot: 'running', pulse: false, label: null, tone: 'running', unseen: false } }),
    go('a1', 'old experiment', { detail: 'no tab', icon: 'agent', state: { dot: 'ended', pulse: false, label: 'Ended', tone: 'muted', unseen: false } }),
    go('s1', 'Settings · PersonalDevTeam', { icon: 'settings', keywords: ['PDT'] }),
    go('s2', 'Settings · This host', { icon: 'host' }),
    act('n1', 'New Claude here', { icon: 'claude' }),
    act('n2', 'New Codex here', { icon: 'codex' }),
    act('n3', 'New shell here', { icon: 'shell' }),
    act('run', 'Run…', { icon: 'run', shortcut: 'Ctrl+] c' }),
    act('rename', 'Rename tab', { icon: 'rename', shortcut: 'Ctrl+] r' }),
    act('close', 'Close tab', { icon: 'close', shortcut: 'Ctrl+] x' }),
    {
      id: 'move', label: 'Move tab to workspace…', group: 'Actions', icon: 'move',
      next: {
        kind: 'list', title: 'Move tab to workspace', placeholder: 'Filter workspaces…',
        items: [
          { id: 'm1', label: 'Scratch', group: 'Workspaces', icon: 'workspace', run: log('Moved to Scratch') },
          {
            id: 'm2', label: 'New workspace…', group: 'Workspaces', icon: 'add',
            next: { kind: 'input', title: 'New workspace', label: 'Name', placeholder: 'Workspace name', submitLabel: 'Create and move', run: (v) => log(`Moved to new workspace "${v}"`)() },
          },
        ],
      },
    },
    act('needs', 'Next needs you', { icon: 'needsYou', shortcut: 'Ctrl+] a' }),
    act('th1', 'Theme: System', { icon: 'check' }),
    act('th2', 'Theme: Light', { icon: 'theme' }),
    act('th3', 'Theme: Dark', { icon: 'theme' }),
    act('keys', 'Keyboard shortcuts', { icon: 'keys', shortcut: 'Ctrl+] ?' }),
  ];
}

function PaletteDemo() {
  const [open, setOpen] = useState(false);
  const [last, setLast] = useState('—');
  const log = (s: string) => () => setLast(s);
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-3">
        <Button onClick={() => setOpen(true)}>Open command palette</Button>
        <span className="text-meta text-muted-foreground">
          In the app: <Kbd keys={['Ctrl', ']']} /> <Kbd>k</Kbd>, or <Kbd>⌘K</Kbd> on a Mac.
        </span>
      </div>
      <p className="text-meta text-muted-foreground">Last: {last}</p>
      <CommandPalette open={open} onOpenChange={setOpen} items={items(log)} />
    </div>
  );
}

export const section = { title: 'Command palette', order: 60, render: () => <PaletteDemo /> };
