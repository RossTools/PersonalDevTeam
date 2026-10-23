import { useId, useState, type FormEvent } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { UserRound } from 'lucide-react';
import {
  Alert,
  Badge,
  Button,
  Checkbox,
  DiffViewer,
  EmptyState,
  Input,
  Page,
  RelativeTime,
  Section,
  SegmentedControl,
  Select,
  SettingsRow,
  Skeleton,
  Textarea,
  useConfirm,
} from '@ui';
import { api } from '../api/client';
import type { Persona } from '../api/gen/Persona';
import type { PersonaSetup } from '../api/gen/PersonaSetup';
import type { PersonaVersion } from '../api/gen/PersonaVersion';
import { Markdown } from '../common/Markdown';
import { useAction } from '../common/useAction';
import { PERMISSION_LABEL, PERMISSION_MODES } from '../conversation/ConversationComposer';
import {
  AGENT_LABEL,
  AGENTS,
  AUTHOR_LABEL,
  DEFAULT_MODE,
  cleanSetup,
  normalTools,
  orderTools,
  personaKeys,
  sameSetup,
  setupOf,
  shownTools,
  sortPersonas,
  usePersonaTools,
  usePersonas,
  type ModeChoice,
} from './model';

const MAX_NAME = 60;

/** A form's Save with its "Saved" note, for a section footer. */
function SaveFooter({ label, text = 'Save', dirty, saved, pending }: { label: string; text?: string; dirty: boolean; saved: boolean; pending: boolean }) {
  return (
    <>
      {saved && !dirty && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      <Button type="submit" variant="primary" size="sm" aria-label={label} disabled={!dirty} pending={pending}>{text}</Button>
    </>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// The setup fields (one version): agent, model, permission mode, tools, instructions

function SetupFields({ value, onChange }: { value: PersonaSetup; onChange: (s: PersonaSetup) => void }) {
  const agentId = useId();
  const modelId = useId();
  const modeId = useId();
  const [preview, setPreview] = useState<'write' | 'preview'>('write');
  const set = (p: Partial<PersonaSetup>) => onChange({ ...value, ...p });
  const known = usePersonaTools().data;
  const checked = shownTools(value.tools);
  const toggle = (name: string, on: boolean) =>
    set({ tools: normalTools(orderTools(on ? [...checked, name] : checked.filter((t) => t !== name), known ?? [])) });
  // Tools the persona has that the server doesn't list stay checked (and saved) until unchecked.
  const unknown = known ? checked.filter((n) => !known.some((t) => t.name === n)) : [];
  return (
    <>
      <SettingsRow label="Agent" htmlFor={agentId}>
        <Select
          id={agentId}
          aria-label="Agent"
          className="w-40"
          options={AGENTS.map((a) => ({ value: a, label: AGENT_LABEL[a] }))}
          value={value.agent}
          onChange={(agent) => set({ agent })}
        />
      </SettingsRow>
      <SettingsRow label="Model" htmlFor={modelId} description="Blank: the agent's default model.">
        <Input
          id={modelId}
          aria-label="Model"
          mono
          className="w-64 max-w-full"
          placeholder="the agent's default"
          value={value.model ?? ''}
          onChange={(e) => set({ model: e.target.value })}
        />
      </SettingsRow>
      <SettingsRow
        label="Permission mode"
        htmlFor={modeId}
        description="A project's own mode for workflow steps wins over this one."
      >
        <Select<ModeChoice>
          id={modeId}
          aria-label="Permission mode"
          className="w-56 max-w-full"
          options={[
            { value: DEFAULT_MODE, label: 'Project or host default' },
            ...PERMISSION_MODES.map((m) => ({ value: m, label: PERMISSION_LABEL[m] })),
          ]}
          value={value.permissionMode ?? DEFAULT_MODE}
          onChange={(m) => set({ permissionMode: m === DEFAULT_MODE ? null : m })}
        />
      </SettingsRow>
      <SettingsRow
        stacked
        label="Tools"
        description="PDT's tools its sessions get. The first five are the default set (what a persona that picks none gets)."
      >
        {known ? (
          <ul className="flex flex-col gap-1.5">
            {known.map((t) => (
              <li key={t.name}>
                <Checkbox checked={checked.includes(t.name)} onChange={(on) => toggle(t.name, on)}>
                  <span className="font-mono text-xs">{t.name}</span>
                  <span className="text-xs text-muted-foreground"> · {t.does}{t.asks && ' (asks you first)'}</span>
                </Checkbox>
              </li>
            ))}
            {unknown.map((n) => (
              <li key={n}>
                <Checkbox checked onChange={(on) => toggle(n, on)}>
                  <span className="font-mono text-xs">{n}</span>
                  <span className="text-xs text-muted-foreground"> · not a tool this PDT has</span>
                </Checkbox>
              </li>
            ))}
          </ul>
        ) : (
          <Skeleton lines={4} label="Loading the tools" />
        )}
      </SettingsRow>
      <SettingsRow stacked label="Instructions" description="Markdown, added to its sessions' instructions.">
        <SegmentedControl
          aria-label="Instructions view"
          size="sm"
          className="self-start"
          options={[{ value: 'write', label: 'Write' }, { value: 'preview', label: 'Preview' }]}
          value={preview}
          onChange={setPreview}
        />
        {preview === 'write' ? (
          <Textarea
            aria-label="Instructions"
            mono
            rows={14}
            spellCheck={false}
            value={value.instructions}
            onChange={(e) => set({ instructions: e.target.value })}
            className="text-xs"
          />
        ) : value.instructions.trim() ? (
          <div className="max-h-96 overflow-auto rounded-md border border-border p-3">
            <Markdown>{value.instructions}</Markdown>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">No instructions.</p>
        )}
      </SettingsRow>
    </>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// New persona

const BLANK: PersonaSetup = { agent: 'claude', model: null, instructions: '', permissionMode: null, tools: [] };

function NewPersonaForm({ onSelect }: { onSelect: (id: string) => void }) {
  const qc = useQueryClient();
  const nameId = useId();
  const roleId = useId();
  const descId = useId();
  const [name, setName] = useState('');
  const [role, setRole] = useState('');
  const [description, setDescription] = useState('');
  const [setup, setSetup] = useState<PersonaSetup>(BLANK);
  const [create, creating] = useAction(async () => {
    const p = await api.createPersona({ name: name.trim(), role: role.trim(), description, ...cleanSetup(setup) });
    await qc.invalidateQueries({ queryKey: personaKeys.all });
    onSelect(p.id);
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (name.trim()) void create();
  };
  return (
    <Page title="New persona" description="An agent setup that workflow steps run: its agent, model, permission mode, tools and instructions.">
      <form onSubmit={submit} aria-label="New persona" className="flex flex-col gap-8">
        <Section label="Persona">
          <SettingsRow label="Name" htmlFor={nameId}>
            <Input id={nameId} aria-label="Name" required maxLength={MAX_NAME} className="w-64 max-w-full" value={name} onChange={(e) => setName(e.target.value)} />
          </SettingsRow>
          <SettingsRow label="Role" htmlFor={roleId} description="Free text: refiner, coder, reviewer, qa…">
            <Input id={roleId} aria-label="Role" className="w-64 max-w-full" value={role} onChange={(e) => setRole(e.target.value)} />
          </SettingsRow>
          <SettingsRow stacked label="Description" htmlFor={descId}>
            <Textarea id={descId} aria-label="Description" rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
          </SettingsRow>
        </Section>
        <Section
          label="Setup"
          footer={<Button type="submit" variant="primary" size="sm" disabled={!name.trim()} pending={creating}>Create persona</Button>}
        >
          <SetupFields value={setup} onChange={setSetup} />
        </Section>
      </form>
    </Page>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// An existing persona

/** Name, role and description: one form, PATCH on Save. */
function Identity({ persona }: { persona: Persona }) {
  const qc = useQueryClient();
  const nameId = useId();
  const roleId = useId();
  const descId = useId();
  const [name, setName] = useState(persona.name);
  const [role, setRole] = useState(persona.role);
  const [description, setDescription] = useState(persona.description);
  const [saved, setSaved] = useState(false);
  const dirty = name.trim() !== persona.name || role.trim() !== persona.role || description !== persona.description;
  const [save, saving] = useAction(async () => {
    await api.patchPersona(persona.id, {
      ...(name.trim() !== persona.name ? { name: name.trim() } : {}),
      ...(role.trim() !== persona.role ? { role: role.trim() } : {}),
      ...(description !== persona.description ? { description } : {}),
    });
    await qc.invalidateQueries({ queryKey: personaKeys.all });
    setSaved(true);
  });
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (dirty && name.trim()) void save(); }}>
      <Section label="Persona" footer={<SaveFooter label="Save persona" dirty={dirty} saved={saved} pending={saving} />}>
        <SettingsRow label="Name" htmlFor={nameId}>
          <Input id={nameId} aria-label="Name" required maxLength={MAX_NAME} className="w-64 max-w-full" value={name} onChange={(e) => { setName(e.target.value); setSaved(false); }} />
        </SettingsRow>
        <SettingsRow label="Role" htmlFor={roleId} description="Free text: refiner, coder, reviewer, qa…">
          <Input id={roleId} aria-label="Role" className="w-64 max-w-full" value={role} onChange={(e) => { setRole(e.target.value); setSaved(false); }} />
        </SettingsRow>
        <SettingsRow stacked label="Description" htmlFor={descId}>
          <Textarea id={descId} aria-label="Description" rows={2} value={description} onChange={(e) => { setDescription(e.target.value); setSaved(false); }} />
        </SettingsRow>
      </Section>
    </form>
  );
}

/** The current version's setup; Save makes a new version (with an optional note). */
function Setup({ persona }: { persona: Persona }) {
  const qc = useQueryClient();
  const noteId = useId();
  const [setup, setSetup] = useState<PersonaSetup>(() => setupOf(persona));
  const [note, setNote] = useState('');
  const dirty = !sameSetup(setup, setupOf(persona));
  const [save, saving] = useAction(async () => {
    await api.savePersona(persona.id, { note: note.trim(), ...cleanSetup(setup) });
    await qc.invalidateQueries({ queryKey: personaKeys.all });
  });
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (dirty) void save(); }}>
      <Section
        label={`Setup · version ${persona.version}`}
        description={
          persona.builtin
            ? "Saving makes a new version. PDT brings a built-in persona up to its newer templates, except the fields you changed."
            : 'Saving makes a new version; sessions record the version they ran with.'
        }
        footer={<SaveFooter label="Save a new version" text="Save new version" dirty={dirty} saved={false} pending={saving} />}
      >
        {persona.builtin && <TemplateNote persona={persona} onLoad={(s) => setSetup(setupOf(s))} />}
        <SetupFields value={setup} onChange={setSetup} />
        <SettingsRow label="Note" htmlFor={noteId} description="Optional: what this version changes.">
          <Input id={noteId} aria-label="Version note" className="w-72 max-w-full" value={note} onChange={(e) => setNote(e.target.value)} />
        </SettingsRow>
      </Section>
    </form>
  );
}

/** A built-in persona whose version differs from PDT's current template (fields the user changed): what differs, and
 * loading the template into the form to review and save. */
function TemplateNote({ persona, onLoad }: { persona: Persona; onLoad: (s: PersonaSetup) => void }) {
  const template = useQuery({ queryKey: personaKeys.template(persona.id, persona.version), queryFn: () => api.personaTemplate(persona.id) });
  const [open, setOpen] = useState(false);
  const t = template.data;
  if (!t || t.diff.changed.length === 0) return null;
  return (
    <div role="region" aria-label="PDT's template" className="flex flex-col gap-2 px-4 py-3 text-xs">
      <p className="flex flex-wrap items-center gap-1.5 text-muted-foreground">
        PDT's template differs from this version in {t.diff.changed.map((f) => <Badge key={f}>{f}</Badge>)}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => setOpen(!open)}>{open ? 'Hide the changes' : 'Show the changes'}</Button>
        <Button size="sm" onClick={() => onLoad(t.setup)}>Load PDT's template</Button>
      </div>
      {open && <DiffViewer diff={t.diff.unified} className="max-h-96 overflow-auto" />}
    </div>
  );
}

/** Newest first; tick two to compare them. */
function Versions({ persona }: { persona: Persona }) {
  const versions = useQuery({ queryKey: personaKeys.versions(persona.id), queryFn: () => api.personaVersions(persona.id) });
  const list = versions.data ?? [];
  // Null until the user picks: the current version against the one before.
  const [picked, setPicked] = useState<number[] | null>(null);
  const picks = picked ?? (list.length >= 2 ? [list[1].version, list[0].version] : []);
  const toggle = (v: number, on: boolean) => setPicked(on ? [...picks.filter((p) => p !== v), v].slice(-2) : picks.filter((p) => p !== v));
  const from = picks.length === 2 ? Math.min(...picks) : null;
  const to = picks.length === 2 ? Math.max(...picks) : null;
  return (
    <Section label="Versions" description="Newest first. Tick two to see what changed between them.">
      {versions.isPending && <Skeleton lines={3} label="Loading versions" />}
      {versions.isError && <Alert tone="error">Versions could not be loaded: {versions.error.message}</Alert>}
      {list.length > 0 && (
        <ul data-flush aria-label="Versions" className="flex flex-col divide-y divide-border">
          {list.map((v) => <VersionRow key={v.version} v={v} current={v.version === persona.version} picked={picks.includes(v.version)} onPick={(on) => toggle(v.version, on)} />)}
        </ul>
      )}
      {from !== null && to !== null && <VersionDiff personaId={persona.id} from={from} to={to} />}
    </Section>
  );
}

function VersionRow({ v, current, picked, onPick }: { v: PersonaVersion; current: boolean; picked: boolean; onPick: (on: boolean) => void }) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-2 text-xs">
      <Checkbox checked={picked} onChange={onPick} aria-label={`Compare version ${v.version}`} />
      <span className="font-medium text-foreground">v{v.version}</span>
      {current && <Badge tone="accent">Current</Badge>}
      <span className="text-muted-foreground">{AUTHOR_LABEL[v.author]}</span>
      <span className="min-w-0 flex-1 truncate" title={v.note || undefined}>{v.note || <span className="text-muted-foreground">No note</span>}</span>
      <RelativeTime time={v.createdAt} className="text-meta text-muted-foreground" />
    </li>
  );
}

function VersionDiff({ personaId, from, to }: { personaId: string; from: number; to: number }) {
  const diff = useQuery({ queryKey: personaKeys.diff(personaId, from, to), queryFn: () => api.personaDiff(personaId, from, to) });
  return (
    <div aria-label={`Changes from version ${from} to ${to}`} role="region" className="flex flex-col gap-2">
      <p className="text-xs font-medium text-foreground">Version {from} → {to}</p>
      {diff.isPending && <Skeleton lines={3} label="Loading the changes" />}
      {diff.isError && <Alert tone="error">The changes could not be loaded: {diff.error.message}</Alert>}
      {diff.data && (
        diff.data.changed.length === 0 ? (
          <p className="text-xs text-muted-foreground">No changes.</p>
        ) : (
          <>
            <p className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
              Changed: {diff.data.changed.map((f) => <Badge key={f}>{f}</Badge>)}
            </p>
            <DiffViewer diff={diff.data.unified} className="max-h-96 overflow-auto" />
          </>
        )
      )}
    </div>
  );
}

function Archive({ persona }: { persona: Persona }) {
  const qc = useQueryClient();
  const [toggle, pending] = useAction(async () => {
    await api.patchPersona(persona.id, { archived: !persona.archived });
    await qc.invalidateQueries({ queryKey: personaKeys.all });
  });
  return (
    <Section label="Archive">
      <SettingsRow
        label={persona.archived ? 'This persona is archived' : 'Archive this persona'}
        description="Archived personas are listed last; their versions stay."
      >
        <Button size="sm" pending={pending} onClick={() => void toggle()}>{persona.archived ? 'Unarchive' : 'Archive'}</Button>
      </SettingsRow>
    </Section>
  );
}

function Delete({ persona, onDeleted }: { persona: Persona; onDeleted: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [remove, removing] = useAction(async () => {
    const { ok } = await confirm({
      title: `Delete ${persona.name}?`,
      body: 'The persona and all its versions are deleted. A persona a workflow runs cannot be deleted: archive it instead.',
      action: 'Delete persona',
      danger: true,
    });
    if (!ok) return;
    await api.deletePersona(persona.id);
    await qc.invalidateQueries({ queryKey: personaKeys.all });
    onDeleted();
  });
  return (
    <Section label="Danger zone" danger>
      <SettingsRow label="Delete this persona" description="Deletes it and its versions.">
        <Button size="sm" variant="danger" pending={removing} onClick={() => void remove()}>Delete persona…</Button>
      </SettingsRow>
    </Section>
  );
}

function PersonaDetail({ persona, onSelect, others }: { persona: Persona; onSelect: (id: string) => void; others: Persona[] }) {
  return (
    <Page
      title={persona.name}
      description={persona.description || undefined}
      actions={
        <>
          {persona.builtin && <Badge>Built in</Badge>}
          {persona.archived && <Badge tone="waiting">Archived</Badge>}
        </>
      }
    >
      <Identity key={`${persona.id}:${persona.name}:${persona.role}:${persona.description}`} persona={persona} />
      <Setup key={`${persona.id}:${persona.version}`} persona={persona} />
      <Versions key={persona.id} persona={persona} />
      <Archive persona={persona} />
      {!persona.builtin && <Delete persona={persona} onDeleted={() => onSelect(others[0]?.id ?? 'new')} />}
    </Page>
  );
}

/**
 * The Personas view's main area: nothing chosen → a hint; `'new'` → the create form; a persona's id → its page
 * (name, role, description; the current setup, saved as a new version; the versions and a diff of any two; archive;
 * delete for those not built in).
 */
export function PersonaPage({ personaId, onSelect }: { personaId: string | null; onSelect: (id: string) => void }) {
  const personas = usePersonas();
  if (personaId === null) return <EmptyState icon={<UserRound />}>Choose a persona</EmptyState>;
  if (personaId === 'new') return <NewPersonaForm onSelect={onSelect} />;
  if (personas.isPending) return <Skeleton lines={6} label="Loading the persona" className="p-6" />;
  if (personas.isError) return <EmptyState role="alert">Personas could not be loaded: {personas.error.message}</EmptyState>;
  const persona = personas.data.find((p) => p.id === personaId);
  if (!persona) return <EmptyState icon={<UserRound />}>This persona no longer exists.</EmptyState>;
  const others = sortPersonas(personas.data.filter((p) => p.id !== personaId));
  return <PersonaDetail key={persona.id} persona={persona} onSelect={onSelect} others={others} />;
}
