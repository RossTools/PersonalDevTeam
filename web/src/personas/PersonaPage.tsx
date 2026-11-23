import { useId, useState, type FormEvent } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { UserRound } from 'lucide-react';
import {
  ActionBar,
  Badge,
  Button,
  Checkbox,
  DiffViewer,
  EmptyState,
  Input,
  Page,
  RelativeTime,
  Section,
  Select,
  SettingsRow,
  Skeleton,
  Textarea,
  toast,
  useConfirm,
} from '@ui';
import { api } from '../api/client';
import type { Persona } from '../api/gen/Persona';
import type { PersonaSetup } from '../api/gen/PersonaSetup';
import type { PersonaVersion } from '../api/gen/PersonaVersion';
import { Markdown } from '../common/Markdown';
import { PreviewEdit } from '../common/PreviewEdit';
import { useAgents } from '../common/agents';
import { EmptyPane } from '../common/EmptyPane';
import { useAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { PERMISSION_LABEL, PERMISSION_MODES } from '../conversation/ConversationComposer';
import {
  AUTHOR_LABEL,
  DEFAULT_MODE,
  cleanSetup,
  defaultTools,
  normalTools,
  orderTools,
  personaKeys,
  sameSetup,
  setupOf,
  sortPersonas,
  toolSummary,
  usePersonaTools,
  usePersonas,
  type ModeChoice,
} from './model';
import { LoadError } from '../common/QueryStatus';
import { ToolsDialog } from './ToolsDialog';

const MAX_NAME = 60;

// ---------------------------------------------------------------------------------------------------------------
// The setup fields (one version): agent, model, permission mode, tools, instructions

function SetupFields({ value, onChange }: { value: PersonaSetup; onChange: (s: PersonaSetup) => void }) {
  const agentId = useId();
  const modelId = useId();
  const modeId = useId();
  const [preview, setPreview] = useState<'edit' | 'preview'>('edit');
  const set = (p: Partial<PersonaSetup>) => onChange({ ...value, ...p });
  const known = usePersonaTools().data;
  const agents = useAgents();
  // The persona's agent stays listed even if the host no longer offers it (an ACP agent removed since, or one switched
  // off in Settings → Agents).
  const agentOptions = agents.all.includes(value.agent) ? agents.all : [...agents.all, value.agent];
  const [choosing, setChoosing] = useState(false);
  // Tools the persona has that the server doesn't list stay on (and saved) until turned off.
  const checked = value.tools;
  const toggle = (names: string[], on: boolean) =>
    set({ tools: normalTools(orderTools(on ? [...checked, ...names] : checked.filter((t) => !names.includes(t)), known ?? [])) });
  return (
    <>
      <SettingsRow label="Agent" htmlFor={agentId}>
        <Select
          id={agentId}
          aria-label="Agent"
          className="w-40"
          options={agentOptions.map((a) => ({ value: a, label: agents.on(a) ? agents.label(a) : `${agents.label(a)} · switched off` }))}
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
        label="Tools"
        description="PDT's tools its sessions get; a persona that picks none gets none. A new persona starts with the default set."
      >
        {known ? (
          <>
            <span role="status" aria-label="Tools on" className="text-xs tabular-nums text-muted-foreground">{toolSummary(checked, known)}</span>
            <Button size="sm" onClick={() => setChoosing(true)}>Choose tools…</Button>
          </>
        ) : (
          <Skeleton lines={1} label="Loading the tools" className="w-40" />
        )}
        {choosing && known && <ToolsDialog known={known} checked={checked} onToggle={toggle} onClose={() => setChoosing(false)} />}
      </SettingsRow>
      <SettingsRow stacked label="Instructions" description="Markdown, added to its sessions' instructions.">
        <PreviewEdit aria-label="Instructions view" className="self-start" value={preview} onChange={setPreview} />
        {preview === 'edit' ? (
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
          <EmptyState size="inline">No instructions.</EmptyState>
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
  // Its tools are the default set until the user picks (the server's list comes in after the first render).
  const [picked, setPicked] = useState(false);
  const known = usePersonaTools().data;
  const shown = picked ? setup : { ...setup, tools: defaultTools(known ?? []) };
  const [create, creating] = useAction(async () => {
    const p = await api.createPersona({ name: name.trim(), role: role.trim(), description, ...cleanSetup(shown) });
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
          <SetupFields
            value={shown}
            onChange={(s) => {
              if (s.tools !== shown.tools) setPicked(true);
              setSetup(s);
            }}
          />
        </Section>
      </form>
    </Page>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// An existing persona

/** What the Persona fields are keyed on: a change from elsewhere (or a save) starts them over. */
const identityKey = (p: Pick<Persona, 'id' | 'name' | 'role' | 'description'>) => `${p.id}:${p.name}:${p.role}:${p.description}`;

type IdentityFields = Pick<Persona, 'name' | 'role' | 'description'>;
const identityOf = (p: Persona): IdentityFields => ({ name: p.name, role: p.role, description: p.description });

/** A form's edits of `init()`, started over when `key` changes (the persona changed elsewhere, or was saved): the
 *  value, a setter and a reset. */
function useDraft<T>(key: string, init: () => T): [T, (v: T) => void, () => void] {
  const [draft, setDraft] = useState(() => ({ key, value: init() }));
  let current = draft;
  if (draft.key !== key) {
    current = { key, value: init() };
    setDraft(current);
  }
  return [current.value, (value) => setDraft({ key, value }), () => setDraft({ key, value: init() })];
}

/** Name, role and description. */
function IdentityFieldsRows({ value, onChange }: { value: IdentityFields; onChange: (v: IdentityFields) => void }) {
  const nameId = useId();
  const roleId = useId();
  const descId = useId();
  const set = (p: Partial<IdentityFields>) => onChange({ ...value, ...p });
  return (
    <>
      <SettingsRow label="Name" htmlFor={nameId}>
        <Input id={nameId} aria-label="Name" required maxLength={MAX_NAME} className="w-64 max-w-full" value={value.name} onChange={(e) => set({ name: e.target.value })} />
      </SettingsRow>
      <SettingsRow label="Role" htmlFor={roleId} description="Free text: refiner, coder, reviewer, qa…">
        <Input id={roleId} aria-label="Role" className="w-64 max-w-full" value={value.role} onChange={(e) => set({ role: e.target.value })} />
      </SettingsRow>
      <SettingsRow stacked label="Description" htmlFor={descId}>
        <Textarea id={descId} aria-label="Description" rows={2} value={value.description} onChange={(e) => set({ description: e.target.value })} />
      </SettingsRow>
    </>
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
      <LoadError what="Versions" error={versions.error} />
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
      <LoadError what="The changes" error={diff.error} />
      {diff.data && (
        diff.data.changed.length === 0 ? (
          <EmptyState size="inline">No changes.</EmptyState>
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

/**
 * An existing persona: its Persona and Setup fields are one form, saved from the bar that sticks to the bottom of the
 * page while either has changes. Name, role and description are changed in place (PATCH); a changed setup is saved as
 * a new version, with the bar's note.
 */
function PersonaDetail({ persona, onSelect, others }: { persona: Persona; onSelect: (id: string) => void; others: Persona[] }) {
  const qc = useQueryClient();
  const [identity, setIdentity, resetIdentity] = useDraft(identityKey(persona), () => identityOf(persona));
  const [setup, setSetup, resetSetup] = useDraft(String(persona.version), () => setupOf(persona));
  const [note, setNote, resetNote] = useDraft(String(persona.version), () => '');
  const patch = {
    ...(identity.name.trim() !== persona.name ? { name: identity.name.trim() } : {}),
    ...(identity.role.trim() !== persona.role ? { role: identity.role.trim() } : {}),
    ...(identity.description !== persona.description ? { description: identity.description } : {}),
  };
  const identityDirty = Object.keys(patch).length > 0;
  const setupDirty = !sameSetup(setup, setupOf(persona));
  const [save, saving] = useAction(async () => {
    if (identityDirty) await api.patchPersona(persona.id, patch);
    const saved = setupDirty ? await api.savePersona(persona.id, { note: note.trim(), ...cleanSetup(setup) }) : null;
    await qc.invalidateQueries({ queryKey: personaKeys.all });
    toast.success(saved ? `Saved as version ${saved.version}` : 'Saved');
  });
  const discard = () => {
    resetIdentity();
    resetSetup();
    resetNote();
  };
  const what = [identityDirty && 'Persona', setupDirty && 'Setup'].filter(Boolean).join(' and ');
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
      {/* contents: the sections stay the page's own, so the bar sticks to the bottom of the page, not of the form. */}
      <form aria-label="Edit persona" className="contents" onSubmit={(e) => { e.preventDefault(); if (what && identity.name.trim()) void save(); }}>
        <Section label="Persona">
          <IdentityFieldsRows value={identity} onChange={setIdentity} />
        </Section>
        <Section
          label={`Setup · version ${persona.version}`}
          description={
            persona.builtin
              ? "Saving it makes a new version. PDT brings a built-in persona up to its newer templates, except the fields you changed."
              : 'Saving it makes a new version; sessions record the version they ran with.'
          }
        >
          {persona.builtin && <TemplateNote persona={persona} onLoad={(s) => setSetup(setupOf(s))} />}
          <SetupFields value={setup} onChange={setSetup} />
        </Section>
        <Versions key={persona.id} persona={persona} />
        <Archive persona={persona} />
        {!persona.builtin && <Delete persona={persona} onDeleted={() => onSelect(others[0]?.id ?? 'new')} />}
        {what && (
          <ActionBar
            aria-label="Unsaved changes"
            className="sticky bottom-0 rounded-lg border shadow-popover"
            end={
              <>
                <Button size="sm" onClick={discard}>Discard</Button>
                <Button type="submit" variant="primary" size="sm" disabled={!identity.name.trim()} pending={saving}>
                  {setupDirty ? 'Save new version' : 'Save'}
                </Button>
              </>
            }
          >
            <span className="text-xs font-medium text-foreground">Unsaved changes to {what}</span>
            {setupDirty && (
              <Input
                aria-label="Version note"
                size="sm"
                placeholder="Note: what this version changes (optional)"
                className="w-72 max-w-full"
                value={note}
                onChange={(e) => setNote(e.target.value)}
              />
            )}
          </ActionBar>
        )}
      </form>
    </Page>
  );
}

/** A persona to read (phones, PDT-120: personas are edited on a computer): who it is, its setup and its versions. */
function PersonaReadOnly({ persona }: { persona: Persona }) {
  const known = usePersonaTools().data;
  const agents = useAgents();
  const tools = persona.tools;
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
      <Section label={`Setup · version ${persona.version}`}>
        <SettingsRow label="Role">{persona.role || <span className="text-muted-foreground">None</span>}</SettingsRow>
        <SettingsRow label="Agent">{agents.label(persona.agent)}</SettingsRow>
        <SettingsRow label="Model"><span className="font-mono break-all">{persona.model || "the agent's default"}</span></SettingsRow>
        <SettingsRow label="Permission mode">{persona.permissionMode ? PERMISSION_LABEL[persona.permissionMode] : 'Project or host default'}</SettingsRow>
        <SettingsRow stacked label="Tools">
          {tools.length === 0 && <EmptyState size="inline">None.</EmptyState>}
          <ul aria-label="Tools" className="flex flex-col gap-1">
            {tools.map((name) => {
              const does = known?.find((t) => t.name === name)?.does;
              return (
                <li key={name} className="text-xs">
                  <span className="font-mono">{name}</span>
                  {does && <span className="text-muted-foreground"> · {does}</span>}
                </li>
              );
            })}
          </ul>
        </SettingsRow>
        <SettingsRow stacked label="Instructions">
          {persona.instructions.trim() ? <Markdown className="min-w-0 break-words">{persona.instructions}</Markdown>
            : <EmptyState size="inline">No instructions.</EmptyState>}
        </SettingsRow>
      </Section>
      <Versions key={persona.id} persona={persona} />
    </Page>
  );
}

/**
 * The Personas view's main area: nothing chosen → a hint; `'new'` → the create form; a persona's id → its page
 * (name, role, description; the current setup, saved as a new version; the versions and a diff of any two; archive;
 * delete for those not built in). On a phone a persona is read-only.
 */
export function PersonaPage({ personaId, onSelect }: { personaId: string | null; onSelect: (id: string) => void }) {
  const personas = usePersonas();
  const phone = useMediaQuery(PHONE_QUERY);
  if (personaId === null) {
    return phone ? <EmptyPane icon={<UserRound />}>Pick a persona.</EmptyPane>
      : <EmptyPane icon={<UserRound />} newLabel="New persona" onNew={() => onSelect('new')}>Pick a persona, or make one.</EmptyPane>;
  }
  if (personaId === 'new') return phone ? <EmptyState icon={<UserRound />}>Make personas on a computer.</EmptyState> : <NewPersonaForm onSelect={onSelect} />;
  if (personas.isPending) return <Skeleton lines={6} label="Loading the persona" className="p-6" />;
  if (personas.isError) return <EmptyState role="alert">Personas could not be loaded: {personas.error.message}</EmptyState>;
  const persona = personas.data.find((p) => p.id === personaId);
  if (!persona) return <EmptyState icon={<UserRound />}>This persona no longer exists.</EmptyState>;
  if (phone) return <PersonaReadOnly key={persona.id} persona={persona} />;
  const others = sortPersonas(personas.data.filter((p) => p.id !== personaId));
  return <PersonaDetail key={persona.id} persona={persona} onSelect={onSelect} others={others} />;
}
