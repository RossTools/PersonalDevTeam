import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { File, FileQuestion, Folder, FolderOpen } from 'lucide-react';
import { Alert, Chip, EmptyState, MenuButton, SegmentedControl, Skeleton, TreeView, ViewHeader, type TreeChildren, type TreeNode } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { FileEntry } from '../api/gen/FileEntry';
import type { Project } from '../api/gen/Project';
import { copyToClipboard } from '../common/clipboard';
import { Markdown } from '../common/Markdown';
import { PlacePicker, type Place } from '../common/PlacePicker';
import { CodeViewer } from './LazyCodeViewer';

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
    setOpen,
    toggle: (path: string) => setOpen((s) => { const n = new Set(s); if (n.has(path)) n.delete(path); else n.add(path); return n; }),
    file,
    setFile,
  };
}

export type FolderViewState = ReturnType<typeof useFolderView>;

const STATUS_TONE: Record<string, string> = { M: 'text-waiting', A: 'text-running', '??': 'text-running', D: 'text-failed', R: 'text-waiting' };
const statusLetter = (status: string) => (status === '??' ? 'U' : status.slice(0, 1));
const statusTone = (status: string) => STATUS_TONE[status] ?? STATUS_TONE[status.slice(0, 1)] ?? 'text-muted-foreground';

function StatusMark({ status }: { status: string | null }) {
  if (!status) return null;
  return <span title={`git: ${status}`} className={cn('font-mono text-2xs', statusTone(status))}>{statusLetter(status)}</span>;
}

type FileNode = TreeNode & { entry: FileEntry };

const toNode = (e: FileEntry): FileNode => ({
  id: e.path,
  label: e.name,
  hasChildren: e.dir,
  icon: e.dir ? <Folder size={13} aria-hidden /> : <File size={13} aria-hidden />,
  trailing: <StatusMark status={e.status} />,
  labelClassName: e.status === 'D' ? 'line-through text-muted-foreground' : undefined,
  title: e.path,
  entry: e,
});

/** One folder's entries, as a query (one per folder, cached by place and path). */
function useFolder(place: Place, path: string) {
  return useQuery({
    queryKey: ['files', place.projectId, place.worktreeId, path],
    queryFn: () => api.listFiles(place.projectId, place.worktreeId, path),
  });
}

/** The checkout's files: the top level here, each folder's children loaded when it opens. */
function Files({ state, place }: { state: FolderViewState; place: Place }) {
  const root = useFolder(place, '');
  // Called by TreeView from the open folder's own component, so it may use a hook.
  const useChildren = (node: FileNode): TreeChildren<FileNode> => {
    const q = useFolder(place, node.id);
    if (q.isPending) return { status: 'loading' };
    if (q.isError) return { status: 'error', message: q.error.message };
    return { status: 'ready', items: q.data.map(toNode) };
  };
  if (root.isPending) return <Skeleton lines={6} label="Loading files" className="px-2" />;
  if (root.isError) return <div className="px-1"><Alert tone="error">{root.error.message}</Alert></div>;
  return (
    <TreeView<FileNode>
      aria-label="Files"
      items={root.data.map(toNode)}
      useChildren={useChildren}
      expandedKeys={state.open}
      onExpandedChange={state.setOpen}
      selectedKey={state.file}
      onAction={(n) => state.setFile(n.id)}
      renderEmpty={() => <p className="px-2 py-1 text-xs text-muted-foreground">This checkout has no files.</p>}
    />
  );
}

/** The left column of the Files view: which checkout, then its files (as git sees them, `.gitignore` honoured). */
export function FolderTree({ state }: { state: FolderViewState }) {
  if (!state.place) return <EmptyState icon={<FolderOpen />}>Add a project to browse its files.</EmptyState>;
  return (
    <div className="flex flex-col gap-2 py-1">
      <div className="px-1"><PlacePicker projects={state.projects} place={state.place} onPick={state.pick} /></div>
      <Files key={`${state.place.projectId}:${state.place.worktreeId}`} state={state} place={state.place} />
    </div>
  );
}

/** Markdown files open rendered, with Source a click away. */
const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

/** The selected file, read-only; a Markdown file shows rendered or as source. */
export function FileViewer({ state }: { state: FolderViewState }) {
  const place = state.place;
  const [markdownView, setMarkdownView] = useState<'rendered' | 'source'>('rendered');
  const q = useQuery({
    queryKey: ['file', place?.projectId, place?.worktreeId, state.file],
    queryFn: () => api.fileContent(place!.projectId, place!.worktreeId, state.file!),
    enabled: !!place && !!state.file,
  });
  if (!state.file) return <EmptyState icon={<File />}>Pick a file to view it. Files are read-only here.</EmptyState>;
  if (q.isPending) return <Skeleton lines={8} label="Loading the file" className="p-4" />;
  if (q.isError) return <div className="p-4"><Alert tone="error">{q.error.message}</Alert></div>;
  const f = q.data;
  const markdown = isMarkdown(f.path) && !f.tooLarge && !f.binary && f.text !== null;
  return (
    <div className="flex flex-1 flex-col min-h-0">
      <ViewHeader
        title={
          // Truncates from the left so the file name stays visible (the bidi isolate keeps the path in order).
          <span className="block truncate font-mono [direction:rtl]"><bdi>{f.path}</bdi></span>
        }
        titleTooltip={f.path}
        meta={
          <>
            {f.status && <Chip title={`git: ${f.status}`} className={statusTone(f.status)}>{statusLetter(f.status)}</Chip>}
            <span className="shrink-0">{f.size.toLocaleString()} bytes</span>
          </>
        }
        actions={
          <>
            {markdown && (
              <SegmentedControl<'rendered' | 'source'>
                aria-label="Show"
                size="sm"
                options={[{ value: 'rendered', label: 'Rendered' }, { value: 'source', label: 'Source' }]}
                value={markdownView}
                onChange={setMarkdownView}
              />
            )}
            <MenuButton
              label="File actions"
              size="sm"
              placement="bottom end"
              entries={() => [{ label: 'Copy path', onSelect: () => void copyToClipboard(f.path, 'Path copied') }]}
            />
          </>
        }
      />
      {f.tooLarge ? (
        <EmptyState icon={<FileQuestion />}>This file is over 1 MB, too large to show here.</EmptyState>
      ) : f.binary || f.text === null ? (
        <EmptyState icon={<FileQuestion />}>This is a binary file.</EmptyState>
      ) : markdown && markdownView === 'rendered' ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <Markdown className="mx-auto max-w-3xl p-4">{f.text}</Markdown>
        </div>
      ) : (
        <div className="flex-1 min-h-0"><CodeViewer name={f.path} text={f.text} /></div>
      )}
    </div>
  );
}
