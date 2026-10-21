// Choosing a node kind from the catalog (searchable, grouped by category), and each category's icon.
import type { ReactElement } from 'react';
import { Bot, CircleStop, Hourglass, Play, ShieldQuestion, Split, Zap, type LucideIcon } from 'lucide-react';
import { CommandList, Popover } from '@ui';
import type { Category } from '../api/gen/Category';
import type { KindSpec } from '../api/gen/KindSpec';

export const CATEGORY: Record<Category, { label: string; icon: LucideIcon }> = {
  trigger: { label: 'Triggers', icon: Zap },
  condition: { label: 'Conditions', icon: Split },
  action: { label: 'Actions', icon: Play },
  gate: { label: 'Ask the user', icon: ShieldQuestion },
  wait: { label: 'Waits', icon: Hourglass },
  flow: { label: 'Flow', icon: CircleStop },
  agent: { label: 'Agent steps', icon: Bot },
};

const ORDER: Category[] = ['trigger', 'action', 'agent', 'condition', 'gate', 'wait', 'flow'];

export function KindIcon({ category }: { category: Category | undefined }) {
  const Icon = CATEGORY[category ?? 'action'].icon;
  return <Icon aria-hidden />;
}

export function KindList({ catalog, onPick, triggers = true }: { catalog: KindSpec[]; onPick: (k: KindSpec) => void; triggers?: boolean }) {
  const kinds = [...catalog]
    .filter((k) => triggers || k.category !== 'trigger')
    .sort((a, b) => ORDER.indexOf(a.category) - ORDER.indexOf(b.category));
  return (
    <CommandList
      label="Node kinds"
      placeholder="Find a trigger, action, condition…"
      items={kinds.map((k) => ({
        id: k.id, label: k.title, detail: k.id, group: CATEGORY[k.category].label, keywords: [k.description],
        icon: <KindIcon category={k.category} />,
      }))}
      onAction={(id) => onPick(kinds.find((k) => k.id === id)!)}
      emptyText="No kind matches."
    />
  );
}

/** A button that opens the catalog; `onPick` gets the chosen kind. */
export function KindPicker({ catalog, onPick, trigger, triggers }: {
  catalog: KindSpec[]; onPick: (k: KindSpec) => void; trigger: ReactElement; triggers?: boolean;
}) {
  return (
    <Popover trigger={trigger} label="Add a node" className="w-80 p-0">
      {({ close }) => <KindList catalog={catalog} triggers={triggers} onPick={(k) => { close(); onPick(k); }} />}
    </Popover>
  );
}
