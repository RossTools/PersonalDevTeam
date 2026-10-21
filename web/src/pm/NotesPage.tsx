import { useId, useState, type FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Pin } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Input, Page, RelativeTime, Select, SettingsRow, Skeleton, Switch, Textarea, useConfirm } from '@ui';
import { api } from '../api/client';
import type { PmNote } from '../api/gen/PmNote';
import type { Project } from '../api/gen/Project';
import { Markdown } from '../common/Markdown';
import { useAction } from '../common/useAction';
import { useRefreshPm } from './usePm';

const HOST_WIDE = 'host';

/**
 * The PM's notes (PDT-81): what PDT's data can't tell it, host-wide or per project, searched with full text. Pinned
 * notes go in every new PM session's briefing. The PM keeps them with its tools; the user can add, edit, pin and
 * delete them here.
 */
export function NotesPage() {
  const [q, setQ] = useState('');
  const [editing, setEditing] = useState<PmNote | 'new' | null>(null);
  const notes = useQuery({ queryKey: ['pm-notes', q], queryFn: () => api.pmNotes({ q: q.trim() || undefined, kind: 'note' }), refetchInterval: 2000 });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const keyOf = (id: string | null) => (id ? projects.data?.find((p) => p.id === id)?.key ?? '?' : null);
  return (
    <Page
      title="Notes"
      description="What the PM knows that PDT's data can't tell it: preferences, decisions, people, rules at work. Pinned notes go in every new PM session's briefing."
      actions={<Button size="sm" onClick={() => setEditing('new')}>New note</Button>}
    >
      {editing && (
        <NoteForm
          key={editing === 'new' ? 'new' : editing.id}
          note={editing === 'new' ? null : editing}
          projects={projects.data ?? []}
          onDone={() => setEditing(null)}
        />
      )}
      <Input type="search" aria-label="Search notes" placeholder="Search notes…" value={q} onChange={(e) => setQ(e.target.value)} />
      {notes.isPending && <Skeleton lines={4} label="Loading notes" />}
      {notes.isError && <Alert tone="error">Notes could not be loaded: {notes.error.message}</Alert>}
      {notes.isSuccess && notes.data.length === 0 && (
        <EmptyState>{q.trim() ? 'No note matches.' : 'No notes yet. The PM saves what it learns about how you work; add your own here.'}</EmptyState>
      )}
      <ul className="flex flex-col gap-3" aria-label="Notes">
        {(notes.data ?? []).map((n) => (
          <li key={n.id} className="flex flex-col gap-1 rounded-md border border-border p-3">
            <div className="flex flex-wrap items-center gap-2">
              {n.pinned && <Pin aria-label="Pinned" className="size-3.5 text-muted-foreground" />}
              <span className="min-w-0 flex-1 truncate font-medium">{n.title}</span>
              <Badge>{keyOf(n.projectId) ?? 'Host-wide'}</Badge>
              <RelativeTime time={n.updatedAt} className="text-meta text-muted-foreground" />
              <Button variant="ghost" size="sm" onClick={() => setEditing(n)}>Edit</Button>
            </div>
            {n.body && <div className="line-clamp-6 text-muted-foreground"><Markdown breaks>{n.body}</Markdown></div>}
          </li>
        ))}
      </ul>
    </Page>
  );
}

function NoteForm({ note, projects, onDone }: { note: PmNote | null; projects: Project[]; onDone: () => void }) {
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
    else await api.createPmNote({ title, body, pinned, ...(p ? { project: p } : {}) });
    await refresh();
    onDone();
  });
  const [remove, removing] = useAction(async () => {
    if (!note) return;
    const { ok } = await confirm({ title: `Delete “${note.title}”?`, body: 'The PM no longer sees it.', action: 'Delete note', danger: true });
    if (!ok) return;
    await api.deletePmNote(note.id);
    await refresh();
    onDone();
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
      <Switch checked={pinned} onChange={setPinned}>Pinned: in every briefing</Switch>
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
