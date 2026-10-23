import { useState, type KeyboardEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Pin } from 'lucide-react';
import { Alert, Button, EmptyState, Input, RelativeTime, Select, Skeleton, Textarea } from '@ui';
import { api } from '../api/client';
import { useAction } from '../common/useAction';
import { firstLine, useRefreshPm } from './usePm';

const ALL = 'all';
const HOST_WIDE = 'host';
/** Recent notes the panel lists. */
const LIMIT = 50;

/**
 * The right panel's Notes tab (PDT-126): quick capture of one of the user's own notes (a title and an optional
 * markdown body; ⌘Enter saves), then a search box, a scope (every note, host-wide ones or a project's) and the
 * recent ones. A click opens the note in `NotePage` (`onOpen`).
 */
export function NotesPanel({ onOpen }: { onOpen: (id: string) => void }) {
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [q, setQ] = useState('');
  const [scope, setScope] = useState(ALL);
  const refresh = useRefreshPm();
  const notes = useQuery({
    queryKey: ['pm-notes', 'user', q, 'panel'],
    queryFn: () => api.pmNotes({ q: q.trim() || undefined, kind: 'user', limit: LIMIT }),
    refetchInterval: 2000,
  });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const keyOf = (id: string | null) => (id ? projects.data?.find((p) => p.id === id)?.key ?? '?' : 'Host-wide');
  const shown = (notes.data ?? []).filter((n) => scope === ALL || (scope === HOST_WIDE ? n.projectId === null : n.projectId === scope));
  const [save, saving] = useAction(async () => {
    if (!title.trim()) return;
    const project = scope === ALL || scope === HOST_WIDE ? {} : { project: scope };
    await api.createPmNote({ title, body, kind: 'user', ...project });
    setTitle('');
    setBody('');
    await refresh();
  });
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    }
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
      <form
        aria-label="New note"
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Input aria-label="Note title" placeholder="A note…" maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} onKeyDown={onKeyDown} />
        <Textarea aria-label="Note text" rows={3} placeholder="Markdown (optional)" value={body} onChange={(e) => setBody(e.target.value)} onKeyDown={onKeyDown} />
        <Button type="submit" size="sm" className="self-end" disabled={!title.trim()} pending={saving}>Save note</Button>
      </form>
      <div className="flex flex-col gap-2">
        <Input type="search" aria-label="Search my notes" placeholder="Search…" value={q} onChange={(e) => setQ(e.target.value)} />
        <Select
          aria-label="Scope"
          options={[
            { value: ALL, label: 'Everywhere' },
            { value: HOST_WIDE, label: 'Host-wide' },
            ...(projects.data ?? []).map((p) => ({ value: p.id, label: `${p.name} (${p.key})` })),
          ]}
          value={scope}
          onChange={setScope}
        />
      </div>
      {notes.isPending && <Skeleton lines={3} label="Loading notes" />}
      {notes.isError && <Alert tone="error">Notes could not be loaded: {notes.error.message}</Alert>}
      {notes.isSuccess && shown.length === 0 && <EmptyState>{q.trim() ? 'No note matches.' : 'No notes here yet.'}</EmptyState>}
      <ul className="flex flex-col gap-1" aria-label="My notes">
        {shown.map((n) => (
          <li key={n.id}>
            <button
              type="button"
              onClick={() => onOpen(n.id)}
              className="flex w-full flex-col gap-0.5 rounded-sm p-2 text-left outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch"
            >
              <span className="flex items-center gap-1.5">
                {n.pinned && <Pin aria-label="Pinned" className="size-3.5 shrink-0 text-muted-foreground" />}
                <span className="min-w-0 flex-1 truncate font-medium">{n.title}</span>
              </span>
              {n.body && <span className="truncate text-muted-foreground">{firstLine(n.body)}</span>}
              <span className="text-meta text-muted-foreground">
                {keyOf(n.projectId)} · <RelativeTime time={n.updatedAt} />
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
