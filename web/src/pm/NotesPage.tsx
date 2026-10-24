import { useId, useState, type FormEvent, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Pin } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Input, Page, RelativeTime, SegmentedControl, Select, SettingsRow, Skeleton, Switch, Textarea, useConfirm } from '@ui';
import { api, ApiError } from '../api/client';
import type { PmNote } from '../api/gen/PmNote';
import type { Project } from '../api/gen/Project';
import { Markdown } from '../common/Markdown';
import { useAction } from '../common/useAction';
import { routePath } from '../shell/route';
import { NewTicketDialog } from '../tickets/NewTicketDialog';
import { firstLine, useRefreshPm } from './usePm';

const HOST_WIDE = 'host';

/** Whose notes a list shows (PDT-126): the user's own (`user`) or the PM's (`note`). */
export type Whose = 'user' | 'note';

const WHOSE_OPTIONS: { value: Whose; label: string }[] = [{ value: 'user', label: 'Mine' }, { value: 'note', label: 'PM’s' }];

/**
 * Notes (PDT-81, PDT-126), host-wide or per project, searched with full text. "Mine" are the user's own (ideas and so
 * on): the PM can find them but never briefs, changes or deletes them, and one can become a ticket. "PM's" are what
 * PDT's data can't tell the PM; it keeps them with its tools, the user can add, edit, pin and delete them here, and
 * pinned ones go in every new PM session's briefing.
 */
export function NotesPage({ onOpen, whose: initialWhose = 'user' }: {
  /** Phones: a tap on a note opens it (`NotePage`) instead of editing it in the list. */
  onOpen?: (id: string) => void;
  whose?: Whose;
} = {}) {
  const [q, setQ] = useState('');
  const [whose, setWhose] = useState<Whose>(initialWhose);
  const [editing, setEditing] = useState<PmNote | 'new' | null>(null);
  const [ticketFrom, setTicketFrom] = useState<PmNote | null>(null);
  const notes = useQuery({ queryKey: ['pm-notes', whose, q], queryFn: () => api.pmNotes({ q: q.trim() || undefined, kind: whose }), refetchInterval: 2000 });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const keyOf = (id: string | null) => (id ? projects.data?.find((p) => p.id === id)?.key ?? '?' : null);
  const mine = whose === 'user';
  return (
    <Page
      title="Notes"
      description={mine
        ? 'Your own notes: ideas, plans, anything to keep. The PM can find them when you ask, but never changes them.'
        : "What the PM knows that PDT's data can't tell it: preferences, decisions, people, rules at work. Pinned notes go in every new PM session's briefing."}
      actions={<Button size="sm" onClick={() => setEditing('new')}>New note</Button>}
    >
      <SegmentedControl
        aria-label="Whose notes"
        className="self-start"
        options={WHOSE_OPTIONS}
        value={whose}
        onChange={(w) => {
          setWhose(w);
          setEditing(null);
        }}
      />
      {editing && (
        <NoteForm
          key={editing === 'new' ? `new-${whose}` : editing.id}
          note={editing === 'new' ? null : editing}
          kind={whose}
          projects={projects.data ?? []}
          onDone={() => setEditing(null)}
        />
      )}
      <Input type="search" aria-label="Search notes" placeholder="Search notes…" value={q} onChange={(e) => setQ(e.target.value)} />
      {notes.isPending && <Skeleton lines={4} label="Loading notes" />}
      {notes.isError && <Alert tone="error">Notes could not be loaded: {notes.error.message}</Alert>}
      {notes.isSuccess && notes.data.length === 0 && (
        <EmptyState>
          {q.trim() ? 'No note matches.' : mine ? 'No notes of your own yet. Add one with New note.' : 'No notes yet. The PM saves what it learns about how you work; add your own here.'}
        </EmptyState>
      )}
      <ul className="flex flex-col gap-3" aria-label="Notes">
        {(notes.data ?? []).map((n) => (
          <li key={n.id} className="flex flex-col gap-1 rounded-md border border-border p-3">
            {onOpen ? (
              <button
                type="button"
                onClick={() => onOpen(n.id)}
                className="flex min-h-control-touch flex-col gap-1 rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <NoteHeading note={n} projectKey={keyOf(n.projectId)} />
                {n.body && <span className="line-clamp-2 text-muted-foreground">{firstLine(n.body)}</span>}
              </button>
            ) : (
              <>
                <NoteHeading note={n} projectKey={keyOf(n.projectId)}>
                  {n.kind === 'user' && <Button variant="ghost" size="sm" onClick={() => setTicketFrom(n)}>Make a ticket</Button>}
                  <Button variant="ghost" size="sm" onClick={() => setEditing(n)}>Edit</Button>
                </NoteHeading>
                {n.body && <div className="line-clamp-6 text-muted-foreground"><Markdown breaks>{n.body}</Markdown></div>}
              </>
            )}
          </li>
        ))}
      </ul>
      {ticketFrom && <TicketFromNote note={ticketFrom} projects={projects.data ?? []} onClose={() => setTicketFrom(null)} />}
    </Page>
  );
}

/** A ticket's description drafted from one of the user's notes: its body, then a link back to the note. */
export function ticketDescription(note: PmNote, origin = window.location.origin): string {
  const link = `${origin}${routePath({ view: 'pm', pm: `note:${note.id}` })}`;
  const back = `From the note [${note.title.replace(/[[\]]/g, '\\$&')}](${link}).`;
  return note.body.trim() ? `${note.body.trim()}\n\n${back}` : back;
}

/** "Make a ticket" on one of the user's notes: the new-ticket form as a draft in the note's project, prefilled. */
function TicketFromNote({ note, projects, onClose }: { note: PmNote; projects: Project[]; onClose: () => void }) {
  if (projects.length === 0) return null;
  return (
    <NewTicketDialog
      heading="Make a ticket"
      projects={projects}
      projectId={note.projectId}
      title={note.title}
      description={ticketDescription(note)}
      onClose={onClose}
      onCreated={onClose}
    />
  );
}

function NoteHeading({ note, projectKey, children }: { note: PmNote; projectKey: string | null; children?: ReactNode }) {
  return (
    <span className="flex flex-wrap items-center gap-2">
      {note.pinned && <Pin aria-label="Pinned" className="size-3.5 text-muted-foreground" />}
      <span className="min-w-0 flex-1 truncate font-medium">{note.title}</span>
      <Badge>{projectKey ?? 'Host-wide'}</Badge>
      <RelativeTime time={note.updatedAt} className="text-meta text-muted-foreground" />
      {children}
    </span>
  );
}

/** One note as a screen of its own (`/pm/notes/:id`): all of it, with Edit (and Make a ticket for the user's own). Deleting it calls `onGone`. */
export function NotePage({ id, onGone }: { id: string; onGone: () => void }) {
  const [editing, setEditing] = useState(false);
  const [makingTicket, setMakingTicket] = useState(false);
  const found = useQuery({
    queryKey: ['pm-notes', 'one', id],
    queryFn: () => api.pmNote(id).catch((e: unknown) => {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }),
    refetchInterval: 2000,
  });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const note = found.data ?? undefined;
  const key = note?.projectId ? projects.data?.find((p) => p.id === note.projectId)?.key ?? '?' : null;
  return (
    <Page title={note?.title ?? 'Note'}>
      {found.isPending && <Skeleton lines={4} label="Loading the note" />}
      {found.isError && <Alert tone="error">The note could not be loaded: {found.error.message}</Alert>}
      {found.isSuccess && !note && <EmptyState>This note is gone.</EmptyState>}
      {note && (editing ? (
        <NoteForm note={note} kind={note.kind === 'user' ? 'user' : 'note'} projects={projects.data ?? []} onDone={() => setEditing(false)} onDeleted={onGone} />
      ) : (
        <>
          <NoteHeading note={note} projectKey={key}>
            {note.kind === 'user' && (
              <Button variant="secondary" size="sm" className="pointer-coarse:min-h-control-touch" onClick={() => setMakingTicket(true)}>Make a ticket</Button>
            )}
            <Button variant="secondary" size="sm" className="pointer-coarse:min-h-control-touch" onClick={() => setEditing(true)}>Edit</Button>
          </NoteHeading>
          {note.body ? <Markdown breaks>{note.body}</Markdown> : <p className="text-muted-foreground">No text.</p>}
        </>
      ))}
      {note && makingTicket && <TicketFromNote note={note} projects={projects.data ?? []} onClose={() => setMakingTicket(false)} />}
    </Page>
  );
}

function NoteForm({ note, kind, projects, onDone, onDeleted = onDone }: {
  note: PmNote | null;
  /** A new note's kind; the user's own notes aren't briefed, so their pin only lists them first. */
  kind: Whose;
  projects: Project[];
  onDone: () => void;
  /** After Delete (default `onDone`). */
  onDeleted?: () => void;
}) {
  const titleId = useId();
  const projectId = useId();
  const [title, setTitle] = useState(note?.title ?? '');
  const [body, setBody] = useState(note?.body ?? '');
  const [project, setProject] = useState(note?.projectId ?? HOST_WIDE);
  const [pinned, setPinned] = useState(note?.pinned ?? false);
  const refresh = useRefreshPm();
  const confirm = useConfirm();
  const [save, saving] = useAction(async () => {
    const p = project === HOST_WIDE ? '' : project;
    if (note) await api.patchPmNote(note.id, { title, body, pinned, project: p });
    else await api.createPmNote({ title, body, pinned, ...(kind === 'user' ? { kind } : {}), ...(p ? { project: p } : {}) });
    await refresh();
    onDone();
  });
  const [remove, removing] = useAction(async () => {
    if (!note) return;
    const { ok } = await confirm({ title: `Delete “${note.title}”?`, body: kind === 'user' ? 'It is gone for good.' : 'The PM no longer sees it.', action: 'Delete note', danger: true });
    if (!ok) return;
    await api.deletePmNote(note.id);
    await refresh();
    onDeleted();
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void save();
  };
  return (
    <form onSubmit={submit} aria-label={note ? 'Edit note' : 'New note'} className="flex flex-col gap-3 rounded-md border border-border p-3">
      <SettingsRow label="Title" htmlFor={titleId}>
        <Input id={titleId} aria-label="Title" required maxLength={200} className="w-72 max-w-full" value={title} onChange={(e) => setTitle(e.target.value)} />
      </SettingsRow>
      <SettingsRow label="Project" htmlFor={projectId} description="Host-wide notes go to every project.">
        <Select
          id={projectId}
          aria-label="Project"
          className="w-56 max-w-full"
          options={[{ value: HOST_WIDE, label: 'Host-wide' }, ...projects.map((p) => ({ value: p.id, label: `${p.name} (${p.key})` }))]}
          value={project}
          onChange={setProject}
        />
      </SettingsRow>
      <Textarea aria-label="Note" rows={6} placeholder="Markdown" value={body} onChange={(e) => setBody(e.target.value)} />
      <Switch checked={pinned} onChange={setPinned}>{kind === 'user' ? 'Pinned: listed first' : 'Pinned: in every briefing'}</Switch>
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" pending={saving}>{note ? 'Save' : 'Add note'}</Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>Cancel</Button>
        {note && (
          <Button type="button" variant="danger" size="sm" className="ml-auto" pending={removing} onClick={() => void remove()}>Delete…</Button>
        )}
      </div>
    </form>
  );
}
