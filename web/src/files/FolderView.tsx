import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, File, Folder } from 'lucide-react';
import { api } from '../api/client';
import type { FileEntry } from '../api/gen/FileEntry';
import type { Project } from '../api/gen/Project';
import { PlacePicker, type Place } from '../workspaces/PlacePicker';
import { CodeViewer } from './CodeViewer';

/** The folder view's state: which checkout, which folders are open, which file is shown. */
export function useFolderView(projects: Project[], initial: Place | null) {
  const [picked, setPicked] = useState<Place | null>(null);
  const place = picked ?? initial ?? (projects[0] ? { projectId: projects[0].id, worktreeId: null } : null);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [file, setFile] = useState<string | null>(null);
  return {
    projects,
    place,
    pick: (p: Place) => { setPicked(p); setOpen(new Set()); setFile(null); },
    open,
    toggle: (path: string) => setOpen((s) => { const n = new Set(s); if (n.has(path)) n.delete(path); else n.add(path); return n; }),
    file,
    setFile,
  };
}

export type FolderViewState = ReturnType<typeof useFolderView>;

const STATUS_TONE: Record<string, string> = { M: 'text-waiting', A: 'text-running', '??': 'text-running', D: 'text-failed', R: 'text-waiting' };

function StatusMark({ status }: { status: string | null }) {
  if (!status) return null;
  const label = status === '??' ? 'U' : status.slice(0, 1);
  return <span title={`git: ${status}`} className={`ml-auto font-mono text-2xs ${STATUS_TONE[status] ?? STATUS_TONE[status.slice(0, 1)] ?? 'text-muted-foreground'}`}>{label}</span>;
}

function Dir({ state, place, path, depth }: { state: FolderViewState; place: Place; path: string; depth: number }) {
  const q = useQuery({
    queryKey: ['files', place.projectId, place.worktreeId, path],
    queryFn: () => api.listFiles(place.projectId, place.worktreeId, path),
  });
  if (q.isPending) return <p className="text-xs text-muted-foreground" style={{ paddingLeft: depth * 12 + 8 }}>Loading…</p>;
  if (q.isError) return <p role="alert" className="text-xs text-failed" style={{ paddingLeft: depth * 12 + 8 }}>{q.error.message}</p>;
  return (
    <ul role={depth === 0 ? 'tree' : 'group'} aria-label={depth === 0 ? 'Files' : undefined} className="flex flex-col">
      {q.data.map((e: FileEntry) => {
        const isOpen = state.open.has(e.path);
        return (
          <li key={e.path} role="treeitem" aria-expanded={e.dir ? isOpen : undefined} aria-selected={!e.dir && state.file === e.path}>
            <button
              type="button"
              onClick={() => (e.dir ? state.toggle(e.path) : state.setFile(e.path))}
              className={`w-full flex items-center gap-1 py-0.5 pr-2 rounded-pdt text-left text-sm ${!e.dir && state.file === e.path ? 'bg-accent' : 'hover:bg-surface-2'}`}
              style={{ paddingLeft: depth * 12 + 4 }}
              title={e.path}
            >
              {e.dir ? (isOpen ? <ChevronDown size={12} aria-hidden /> : <ChevronRight size={12} aria-hidden />) : <span className="w-3" />}
              {e.dir ? <Folder size={13} aria-hidden className="text-muted-foreground" /> : <File size={13} aria-hidden className="text-muted-foreground" />}
              <span className={`truncate ${e.status === 'D' ? 'line-through text-muted-foreground' : ''}`}>{e.name}</span>
              <StatusMark status={e.status} />
            </button>
            {e.dir && isOpen && <Dir state={state} place={place} path={e.path} depth={depth + 1} />}
          </li>
        );
      })}
    </ul>
  );
}

/** The left column of the Files view: which checkout, then its files (as git sees them, `.gitignore` honoured). */
export function FolderTree({ state }: { state: FolderViewState }) {
  if (!state.place) return <p className="px-2 text-xs text-muted-foreground">Add a project to browse its files.</p>;
  return (
    <div className="flex flex-col gap-2 py-1">
      <div className="px-1"><PlacePicker projects={state.projects} place={state.place} onPick={state.pick} /></div>
      <Dir key={`${state.place.projectId}:${state.place.worktreeId}`} state={state} place={state.place} path="" depth={0} />
    </div>
  );
}

/** The selected file, read-only. */
export function FileViewer({ state }: { state: FolderViewState }) {
  const place = state.place;
  const q = useQuery({
    queryKey: ['file', place?.projectId, place?.worktreeId, state.file],
    queryFn: () => api.fileContent(place!.projectId, place!.worktreeId, state.file!),
    enabled: !!place && !!state.file,
  });
  if (!state.file) return <p className="p-4 text-sm text-muted-foreground">Pick a file to view it. Files are read-only here.</p>;
  if (q.isPending) return <p className="p-4 text-sm text-muted-foreground">Loading…</p>;
  if (q.isError) return <p role="alert" className="p-4 text-sm text-failed">{q.error.message}</p>;
  const f = q.data;
  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <span className="font-mono truncate" title={f.path}>{f.path}</span>
        <StatusMark status={f.status} />
        <span className="text-xs text-muted-foreground">{f.size.toLocaleString()} bytes</span>
      </header>
      {f.tooLarge ? (
        <p className="p-4 text-sm text-muted-foreground">This file is over 1 MB, too large to show here.</p>
      ) : f.binary || f.text === null ? (
        <p className="p-4 text-sm text-muted-foreground">This is a binary file.</p>
      ) : (
        <div className="flex-1 min-h-0"><CodeViewer name={f.path} text={f.text} /></div>
      )}
    </div>
  );
}
