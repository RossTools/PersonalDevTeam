// New workflow (the gallery, PDT-229): templates by category with search, a preview of the picked
// one (trigger → steps), its params, a name and where it applies. It starts off, so it can be tried before it is
// enabled. PDT's built-in workflows sit in their own collapsed group at the end, to add one back (or copy it).
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { Alert, Badge, Button, Dialog, Field, Input, Link, Select, Sheet, Skeleton, Switch, Textarea } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { ConnectionPicker } from '../connections/ConnectionPicker';
import { DurationInput } from './DurationInput';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { navigate } from '../shell/router';
import { routePath } from '../shell/route';
import type { GalleryTemplate } from '../api/gen/GalleryTemplate';
import type { Project } from '../api/gen/Project';
import type { Scope } from '../api/gen/Scope';
import type { TemplateParam } from '../api/gen/TemplateParam';
import { BLANK, gallery, pluginOff, triggerLabel, type GalleryFilter } from './gallery';
import { invalidateWorkflows, workflowKeys } from './queries';

/** The value a param starts with in the form. */
function initial(p: TemplateParam): unknown {
  if (p.default !== undefined && p.default !== null) return p.default;
  return p.type === 'boolean' ? false : p.type === 'choice' ? (p.choices ?? [])[0] : p.type === 'number' ? '' : '';
}

export function ParamsForm({ params, values, onChange }: { params: TemplateParam[]; values: Record<string, unknown>; onChange: (v: Record<string, unknown>) => void }) {
  const set = (id: string, v: unknown) => onChange({ ...values, [id]: v });
  return (
    <>
      {params.map((p) => {
        const v = values[p.id];
        const label = <>{p.title}{p.required && <span className="text-muted-foreground"> *</span>}</>;
        return (
          <Field key={p.id} label={label} hint={p.description || undefined}>
            {p.type === 'connection' ? <ConnectionPicker label={p.title} kind={p.connectionKind} value={String(v ?? '')} onChange={(n) => set(p.id, n)} />
              : p.type === 'duration' ? <DurationInput label={p.title} value={v} onChange={(ms) => set(p.id, ms)} />
              : p.type === 'boolean' ? <Switch checked={v === true} onChange={(on) => set(p.id, on)} />
              : p.type === 'choice' ? <Select options={(p.choices ?? []).map((c) => ({ value: c, label: c }))} value={String(v ?? '')} onChange={(c) => set(p.id, c)} />
                : p.type === 'text' ? <Textarea rows={3} value={String(v ?? '')} onChange={(e) => set(p.id, e.target.value)} />
                  : <Input type={p.type === 'number' ? 'number' : 'text'} value={Array.isArray(v) ? v.join(', ') : String(v ?? '')}
                    placeholder={p.type === 'list' ? 'Comma-separated' : undefined}
                    onChange={(e) => set(p.id, p.type === 'number' ? (e.target.value === '' ? '' : Number(e.target.value)) : e.target.value)} />}
          </Field>
        );
      })}
    </>
  );
}

const startValues = (params: TemplateParam[]) => Object.fromEntries(params.map((p) => [p.id, initial(p)]));

const groupHeading = 'mt-4 mb-2 flex items-center gap-1.5 text-2xs font-medium uppercase tracking-wider text-muted-foreground first:mt-0';

/** How many steps the preview lists before "and N more". */
const STEPS_SHOWN = 8;

function Card({ t, selected, onPick }: { t: GalleryTemplate; selected: boolean; onPick: () => void }) {
  const off = pluginOff(t);
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onPick}
      className={cn(
        'flex flex-col gap-1.5 rounded-lg border border-border bg-background p-3 text-left outline-none hover:border-muted-foreground focus-visible:ring-2 focus-visible:ring-ring',
        t.id === BLANK && 'border-dashed',
        selected && 'border-primary ring-2 ring-accent',
      )}
    >
      <span className={cn('text-sm font-medium', off && 'text-muted-foreground')}>{t.id === BLANK ? 'Start from scratch' : t.name}</span>
      {t.description && <span className="line-clamp-2 text-xs text-muted-foreground">{t.description}</span>}
      <span className="flex flex-wrap gap-1.5">
        <Badge>{triggerLabel(t)}</Badge>
        {t.plugin && <Badge tone={off ? 'waiting' : 'accent'}>{off ? `${t.plugin.label} is off` : t.plugin.label}</Badge>}
        {t.builtinWorkflow && (t.added ? <Badge tone="running">Added</Badge> : <Badge tone="waiting">Not added</Badge>)}
        {!t.builtin && <Badge>Yours</Badge>}
      </span>
    </button>
  );
}

function Grid({ items, picked, onPick }: { items: GalleryTemplate[]; picked: string; onPick: (id: string) => void }) {
  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-[repeat(auto-fill,minmax(12.5rem,1fr))]">
      {items.map((t) => <Card key={t.id} t={t} selected={t.id === picked} onPick={() => onPick(t.id)} />)}
    </div>
  );
}

/** The picked template: what it does, then the form that creates a workflow from it. */
function Preview({ t, projects, onClose, onCreated, onEditTemplate }: {
  t: GalleryTemplate; projects: Project[]; onClose: () => void; onCreated: (id: string) => void; onEditTemplate?: (id: string) => void;
}) {
  const qc = useQueryClient();
  const [values, setValues] = useState<Record<string, unknown>>(() => startValues(t.params));
  const [name, setName] = useState(t.id === BLANK ? '' : t.doc.name);
  const [where, setWhere] = useState<string>('host');
  const off = pluginOff(t);
  const plugin = t.plugin;
  const create = useMutation({
    meta: { errorShown: true },
    mutationFn: () => {
      const scope: Scope = where === 'host' || where === 'personal' ? where : 'project';
      const filled = Object.fromEntries(Object.entries(values).filter(([, v]) => v !== ''));
      return api.instantiateWorkflowTemplate(t.id, {
        params: filled, name: name.trim() || null, scope, projectId: scope === 'project' ? where : null,
      });
    },
    onSuccess: (w) => {
      invalidateWorkflows(qc);
      void qc.invalidateQueries({ queryKey: workflowKeys.templates });
      onCreated(w.id);
    },
  });
  const openPlugin = (id: string) => { onClose(); navigate(routePath({ view: 'settings', page: { kind: 'plugins', id } })); };
  const steps = t.steps.slice(0, STEPS_SHOWN);
  const node = 'rounded-md border border-border bg-background px-2.5 py-1.5 text-xs';
  const arrow = <div aria-hidden className="ml-4 h-3.5 border-l border-border" />;
  return (
    <section aria-label={`Preview: ${t.name}`} className="flex min-h-full flex-col gap-3">
      <div>
        <h3 className="text-sm font-semibold">{t.id === BLANK ? 'Start from scratch' : t.name}</h3>
        {t.description && <p className="mt-1 text-xs text-muted-foreground">{t.description}</p>}
        {!t.builtin && onEditTemplate && <Link className="mt-1 text-xs" onClick={() => onEditTemplate(t.id)}>Edit this template</Link>}
      </div>
      {off && plugin && (
        <Alert tone="warning">
          Needs the {plugin.label} plugin. <Link onClick={() => openPlugin(plugin.id)}>Turn on {plugin.label}</Link> in Settings → Plugins to use it.
        </Alert>
      )}
      {t.builtinWorkflow && t.added && <Alert tone="info">Already in your workflows. This adds a second copy.</Alert>}
      <div>
        <p className="mb-1 text-xs text-muted-foreground">What it does</p>
        <ol aria-label="What it does" className="flex flex-col">
          <li className={cn(node, 'border-primary')}>
            <span className="font-medium">When</span> {t.triggers.length ? t.triggers.join(', or ') : 'nothing starts it'}
          </li>
          {(steps.length ? steps : ['End']).map((s, i) => <li key={i}>{arrow}<div className={node}>{s}</div></li>)}
          {t.steps.length > STEPS_SHOWN && <li>{arrow}<div className={cn(node, 'text-muted-foreground')}>and {t.steps.length - STEPS_SHOWN} more steps</div></li>}
        </ol>
      </div>
      <ParamsForm params={t.params} values={values} onChange={setValues} />
      <Field label="Name">
        <Input value={name} placeholder={t.doc.name || 'My workflow'} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Project" hint="A project's workflow sees only its events; one for this host sees every project's.">
        <Select
          className="w-full"
          options={[
            { value: 'host', label: 'This host (every project)' },
            ...projects.map((p) => ({ value: p.id, label: p.name })),
            { value: 'personal', label: 'None (personal)' },
          ]}
          value={where}
          onChange={setWhere}
        />
      </Field>
      {create.error && <Alert tone="error">{create.error.message}</Alert>}
      <div className="mt-auto flex justify-end gap-2 pt-2">
        <Button variant="secondary" onClick={onClose}>Cancel</Button>
        <Button variant="primary" disabled={off} title={off ? `Turn on ${t.plugin?.label} first` : undefined} pending={create.isPending} onClick={() => create.mutate()}>
          Create workflow
        </Button>
      </div>
    </section>
  );
}

export function NewWorkflowDialog({ projects, templateId, onClose, onCreated, onEditTemplate }: {
  projects: Project[]; templateId?: string; onClose: () => void; onCreated: (id: string) => void;
  /** Opens one of the user's own templates' page (its params and document). */
  onEditTemplate?: (id: string) => void;
}) {
  const templates = useQuery({ queryKey: workflowKeys.templates, queryFn: () => api.listWorkflowTemplates() });
  const phone = useMediaQuery(PHONE_QUERY);
  const [q, setQ] = useState('');
  const [filter, setFilter] = useState<GalleryFilter>('all');
  const [picked, setPicked] = useState(templateId ?? BLANK);
  // A template asked for by id that is a built-in workflow opens its group.
  const [builtinsOpen, setBuiltinsOpen] = useState(false);
  // On a phone the preview is a sheet over the list, opened by picking a card.
  const [sheet, setSheet] = useState(templateId !== undefined);
  const list = templates.data ?? [];
  const g = gallery(list, q, filter);
  const template = list.find((t) => t.id === picked);
  const showBuiltins = builtinsOpen || filter === 'builtin' || (template?.builtinWorkflow ?? false);
  const pick = (id: string) => { setPicked(id); setSheet(true); };
  const startFromScratch = () => { setQ(''); setFilter('all'); pick(BLANK); };

  const filters: { id: GalleryFilter; label: string; count: number }[] = [
    { id: 'all', label: 'All', count: g.counts.all },
    ...g.counts.categories,
  ];
  const nav = phone ? (
    <div role="group" aria-label="Categories" className="flex gap-1.5 overflow-x-auto pb-2">
      {[...filters, { id: 'builtin' as const, label: 'Built-in', count: g.counts.builtin }].map((f) => (
        <Button key={f.id} size="sm" variant={filter === f.id ? 'primary' : 'secondary'} aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>
          {f.label}
        </Button>
      ))}
    </div>
  ) : (
    <nav aria-label="Categories" className="flex flex-col gap-0.5 overflow-auto border-r border-border pr-2">
      {filters.map((f) => <FilterRow key={f.id} label={f.label} count={f.count} on={filter === f.id} onPick={() => setFilter(f.id)} />)}
      <hr className="mx-1 my-2 border-border" />
      <FilterRow label="Built-in" count={g.counts.builtin} on={filter === 'builtin'} onPick={() => setFilter('builtin')} />
    </nav>
  );

  const body = templates.error ? <Alert tone="error">The templates could not be loaded: {templates.error.message}</Alert>
    : !templates.data ? <Skeleton lines={6} />
      : g.empty ? (
        <p className="py-5 text-sm text-muted-foreground">No templates match. <Link onClick={startFromScratch}>Start from scratch</Link></p>
      ) : (
        <>
          {g.blank && <div className="mb-2"><Grid items={[g.blank]} picked={picked} onPick={pick} /></div>}
          {g.groups.map((grp) => (
            <section key={grp.id} aria-label={grp.label}>
              <h3 className={groupHeading}>{grp.label}</h3>
              <Grid items={grp.items} picked={picked} onPick={pick} />
            </section>
          ))}
          {g.builtins.length > 0 && (
            <section aria-label="Built-in">
              {filter === 'builtin' ? <h3 className={groupHeading}>Built-in</h3> : (
                <h3 className={groupHeading}>
                  <button type="button" aria-expanded={showBuiltins} className="flex items-center gap-1.5 uppercase outline-none focus-visible:ring-2 focus-visible:ring-ring" onClick={() => setBuiltinsOpen(!showBuiltins)}>
                    {showBuiltins ? <ChevronDown aria-hidden className="size-3" /> : <ChevronRight aria-hidden className="size-3" />}
                    Built-in · {g.builtins.length}
                  </button>
                  <span className="normal-case tracking-normal">— PDT's own workflows, to add back one you deleted</span>
                </h3>
              )}
              {showBuiltins && <Grid items={g.builtins} picked={picked} onPick={pick} />}
            </section>
          )}
          {g.groups.length === 0 && !g.blank && g.builtins.length === 0 && <p className="py-5 text-sm text-muted-foreground">No templates here match.</p>}
        </>
      );

  const preview = template && (
    <Preview key={template.id} t={template} projects={projects} onClose={onClose} onCreated={onCreated} onEditTemplate={onEditTemplate} />
  );
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      dismissable={false}
      size="gallery"
      title="New workflow"
      headerActions={<Input aria-label="Search templates" placeholder="Search templates" value={q} onChange={(e) => setQ(e.target.value)} className="flex-1" />}
    >
      {phone ? (
        <div className="flex h-full flex-col">
          {nav}
          <div className="min-h-0 flex-1 overflow-auto">{body}</div>
          {template && (
            <Sheet side="bottom" title={template.id === BLANK ? 'Start from scratch' : template.name} hideTitle open={sheet} onOpenChange={setSheet} className="max-h-[85dvh]">
              {preview}
            </Sheet>
          )}
        </div>
      ) : (
        <div className="grid h-full grid-cols-[11rem_minmax(0,1fr)_20rem] gap-0">
          {nav}
          <div className="min-h-0 overflow-auto px-4">{body}</div>
          <div className="min-h-0 overflow-auto border-l border-border pl-4">{preview}</div>
        </div>
      )}
    </Dialog>
  );
}

function FilterRow({ label, count, on, onPick }: { label: string; count: number; on: boolean; onPick: () => void }) {
  return (
    <button
      type="button"
      aria-current={on ? 'true' : undefined}
      onClick={onPick}
      className={cn(
        'flex w-full items-center rounded-md px-2.5 py-1.5 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring',
        on ? 'bg-accent font-medium' : 'hover:bg-muted',
      )}
    >
      <span className="flex-1">{label}</span>
      <span className="text-muted-foreground">{count || ''}</span>
    </button>
  );
}
