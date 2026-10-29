// Settings → Plugins (PDT-228): the catalog of what this PDT works with. Search, All · On · Needs attention, a chip
// per category, and cards or a list (remembered on this device; a phone always lists). A card opens the plugin's page;
// its switch turns it on or off. A turned-off plugin adds nothing (no connection kinds, workflow steps, PM tools or
// checks); its connections, project settings and workflows are kept for when it is on again.
import { useState } from 'react';
import { LayoutGrid, List as ListIcon } from 'lucide-react';
import { Alert, Button, Input, Page, SegmentedControl, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import {
  CATEGORIES, CATEGORY_LABEL, NO_FILTER, categoryCounts, filterPlugins, groupPlugins, loadLayout, needsAttention, storeLayout,
  type Plugin, type PluginFilter, type PluginLayout, type PluginTab,
} from './model';
import { PluginIcon, PluginStatusBadge, PluginSwitch, usePluginSwitch, usePlugins } from './parts';

interface ItemProps {
  p: Plugin;
  pending: boolean;
  onOpen: () => void;
  onToggle: (on: boolean) => void;
}

function Card({ p, pending, onOpen, onToggle }: ItemProps) {
  return (
    <div
      role="group"
      aria-label={p.label}
      onClick={onOpen}
      className={cn('flex min-h-32 cursor-pointer flex-col gap-2 rounded-lg border border-border bg-card p-3.5 hover:border-primary', !p.built && 'opacity-55')}
    >
      <div className="flex items-center gap-2.5">
        <PluginIcon p={p} />
        <button type="button" className="min-w-0 flex-1 truncate text-left text-sm font-semibold outline-none focus-visible:underline" onClick={(e) => { e.stopPropagation(); onOpen(); }}>
          {p.label}
        </button>
        <PluginSwitch p={p} pending={pending} onChange={onToggle} />
      </div>
      <p className="flex-1 text-xs text-muted-foreground">{p.description || 'This PDT was built without it.'}</p>
      <div><PluginStatusBadge p={p} /></div>
    </div>
  );
}

function Row({ p, pending, onOpen, onToggle }: ItemProps) {
  return (
    <div role="group" aria-label={p.label} onClick={onOpen} className={cn('flex cursor-pointer items-center gap-3 px-3.5 py-2.5 hover:bg-muted', !p.built && 'opacity-55')}>
      <PluginIcon p={p} size="sm" />
      <button type="button" className="w-36 shrink-0 truncate text-left text-sm font-semibold outline-none focus-visible:underline max-sm:flex-1" onClick={(e) => { e.stopPropagation(); onOpen(); }}>
        {p.label}
      </button>
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground max-sm:hidden">{p.description}</span>
      <PluginStatusBadge p={p} />
      <PluginSwitch p={p} pending={pending} onChange={onToggle} />
    </div>
  );
}

export interface PluginsPageProps {
  /** Opens a plugin's page. */
  onOpen: (id: string) => void;
  /** A phone: always list rows. */
  phone?: boolean;
}

export function PluginsPage({ onOpen, phone = false }: PluginsPageProps) {
  const list = usePlugins();
  const all = list.data ?? [];
  const sw = usePluginSwitch(all);
  const [filter, setFilter] = useState<PluginFilter>(NO_FILTER);
  const [layout, setLayout] = useState<PluginLayout>(loadLayout);
  const shown = filterPlugins(all, filter);
  const counts = categoryCounts(all, filter);
  const attention = all.filter(needsAttention).length;
  const set = (f: Partial<PluginFilter>) => setFilter((x) => ({ ...x, ...f }));
  const pickLayout = (l: PluginLayout) => { setLayout(l); storeLayout(l); };
  const asList = phone || layout === 'list';
  const item = (p: Plugin) => {
    const props = { p, pending: sw.pending, onOpen: () => onOpen(p.id), onToggle: (on: boolean) => sw.toggle(p, on) };
    return asList ? <Row key={p.id} {...props} /> : <Card key={p.id} {...props} />;
  };
  return (
    <Page
      title="Plugins"
      className="max-w-5xl"
      description="What this PDT works with. Turn one on and it adds its connections, workflow steps, templates and PM tools. Turning one off keeps its settings, connections and workflows for later."
    >
      {list.error && <Alert tone="error">Plugins could not be loaded: {list.error.message}</Alert>}
      {list.isLoading && <Skeleton label="Loading plugins" className="h-24" />}
      {list.data && (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <Input aria-label="Search plugins" placeholder="Search plugins" className="min-w-48 flex-1" value={filter.q} onChange={(e) => set({ q: e.target.value })} />
            <SegmentedControl<PluginTab>
              aria-label="Show"
              value={filter.tab}
              onChange={(tab) => set({ tab })}
              options={[
                { value: 'all', label: 'All' },
                { value: 'on', label: 'On' },
                { value: 'attention', label: attention ? `Needs attention · ${attention}` : 'Needs attention' },
              ]}
            />
            {!phone && (
              <SegmentedControl<PluginLayout>
                aria-label="Layout"
                value={layout}
                onChange={pickLayout}
                options={[
                  { value: 'cards', label: '', icon: <LayoutGrid className="size-4" />, 'aria-label': 'Cards' },
                  { value: 'list', label: '', icon: <ListIcon className="size-4" />, 'aria-label': 'List' },
                ]}
              />
            )}
          </div>
          <div role="group" aria-label="Categories" className="flex flex-wrap gap-1.5">
            {([null, ...CATEGORIES] as const).map((c) => (
              <Button key={c ?? 'all'} size="sm" variant={filter.category === c ? 'primary' : 'secondary'} aria-pressed={filter.category === c}
                onClick={() => set({ category: c })}>
                {c ? CATEGORY_LABEL[c] : 'All'}{' '}
                <span className="opacity-70">{counts[c ?? 'all']}</span>
              </Button>
            ))}
          </div>
          {shown.length === 0 && <p className="text-sm text-muted-foreground">No plugins match.</p>}
          {groupPlugins(shown).map(([c, ps]) => (
            <section key={c} aria-label={CATEGORY_LABEL[c]} className="flex flex-col gap-2">
              <h3 className="mt-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">{CATEGORY_LABEL[c]}</h3>
              {asList
                ? <div className="flex flex-col divide-y divide-border overflow-clip rounded-lg border border-border bg-card">{ps.map(item)}</div>
                : <div className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-2.5">{ps.map(item)}</div>}
            </section>
          ))}
          <p className="mt-4 rounded-lg border border-dashed border-border px-3.5 py-3 text-xs text-muted-foreground">
            Later: an <b>Available</b> section for plugins installed from outside PDT. Every plugin is built in for now.
          </p>
        </div>
      )}
      {sw.error && <Alert tone="error">{sw.error.message}</Alert>}
      {sw.dialog}
    </Page>
  );
}
