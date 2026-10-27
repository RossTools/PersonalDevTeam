import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronRight, File, FilePlus, FileQuestion, Folder, FolderOpen } from 'lucide-react';
import {
  Alert, Banner, Button, Chip, Dialog, EmptyState, IconButton, List, ListRow, MenuButton, SegmentedControl, Skeleton, toast, TreeView,
  useConfirm, useMenuAt, ViewHeader, type MenuEntry, type TreeChildren, type TreeNode,
} from '@ui';
import { cn } from '@ui/lib/utils';
import { api, ApiError } from '../api/client';
import type { FileContent } from '../api/gen/FileContent';
import type { FileEntry } from '../api/gen/FileEntry';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { copyToClipboard } from '../common/clipboard';
import { MarkdownEditor } from '../common/LazyMarkdownEditor';
import { isMarkdownPath, Markdown } from '../common/Markdown';
import { PlacePicker, type Place } from '../common/PlacePicker';
import { useNavigationGuard, whenAllowed } from '../shell/router';
import { POLL } from '../tickets/queries';
import { CodeViewer } from './LazyCodeViewer';

// The conflict diff is CodeMirror's merge view: loaded only when it is shown.
const CodeDiff = lazy(() => import('../review/CodeDiff').then((m) => ({ default: m.CodeDiff })));

/**
 * The checkout picked and the file shown, when something else holds them (the app keeps them in its route, PDT-111);
 * without it the view keeps its own.
 */
export interface FolderSelection {
  picked: Place | null;
  file: string | null;
  onChange: (picked: Place | null, file: string | null) => void;
}

export function useFolderView(projects: Project[], initial: Place | null, held?: FolderSelection) {
  const [ownPicked, setOwnPicked] = useState<Place | null>(null);
  const [ownFile, setOwnFile] = useState<string | null>(null);
  const picked = held ? held.picked : ownPicked;
  const file = held ? held.file : ownFile;
  // Held: the app's navigation asks about unsaved edits; here, the same guard does.
  const change = (p: Place | null, f: string | null) => {
    if (held) held.onChange(p, f);
    else whenAllowed(() => { setOwnPicked(p); setOwnFile(f); });
  };
  const place = picked ?? initial ?? (projects[0] ? { projectId: projects[0].id, worktreeId: null } : null);
  const placeKey = place ? `${place.projectId}:${place.worktreeId}` : '';
  const [open, setOpen] = useState<Set<string>>(new Set());
  // The file with unsaved edits, if any: it can't be renamed or deleted under the editor.
  const [unsaved, setUnsaved] = useState<string | null>(null);
  // Another checkout starts with its folders closed (not the first one shown once projects load: a linked file's
  // folders are open in it).
  const shownPlace = useRef(placeKey);
  useEffect(() => {
    const was = shownPlace.current;
    shownPlace.current = placeKey;
    if (was && was !== placeKey) setOpen(new Set());
  }, [placeKey]);
  // A file shown from a link or a reload: its folders open, so it is in the tree.
  useEffect(() => {
    const parents = file?.split('/').slice(0, -1).map((_, i, all) => all.slice(0, i + 1).join('/')) ?? [];
    if (parents.some((p) => !open.has(p))) setOpen((s) => new Set([...s, ...parents]));
    // Only when the file changes: folders closed by hand stay closed.
  }, [file]);
  return {
    projects,
    place,
    pick: (p: Place) => change(p, null),
    open,
    setOpen,
    toggle: (path: string) => setOpen((s) => { const n = new Set(s); if (n.has(path)) n.delete(path); else n.add(path); return n; }),
    file,
    setFile: (f: string | null) => change(picked, f),
    unsaved,
    setUnsaved,
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
  const ops = useFileOps(state, place);
  const { openMenu, menu } = useMenuAt();
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
    <>
      <TreeView<FileNode>
        aria-label="Files"
        items={root.data.map(toNode)}
        useChildren={useChildren}
        expandedKeys={state.open}
        onExpandedChange={state.setOpen}
        selectedKey={state.file}
        onAction={(n) => state.setFile(n.id)}
        onMenu={(n, at) => openMenu(at, `${n.entry.name} actions`, () => ops.entries(n.entry))}
        renderEmpty={() => <p className="px-2 py-1 text-xs text-muted-foreground">This checkout has no files.</p>}
      />
      {menu}
    </>
  );
}

/** "New file…": a path inside the checkout (folders are made as needed). */
function NewFileButton({ state, place }: { state: FolderViewState; place: Place }) {
  const ops = useFileOps(state, place);
  return (
    <MenuButton label="New file" entries={() => [{ input: 'New file path…', onSubmit: (path) => void ops.create(path) }]}>
      <IconButton label="New file" size="sm"><FilePlus aria-hidden /></IconButton>
    </MenuButton>
  );
}

const parentOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');

/** Making, renaming and deleting files in the shown checkout; `entries` is a file's or folder's menu. */
function useFileOps(state: FolderViewState, place: Place) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const wt = place.worktreeId ? { worktreeId: place.worktreeId } : {};
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['files', place.projectId, place.worktreeId] });
    void qc.invalidateQueries({ queryKey: ['file', place.projectId, place.worktreeId] });
  };
  const fail = (e: unknown) => toast.error(e instanceof Error ? e.message : String(e));
  const create = async (path: string) => {
    try {
      const made = await api.saveFile(place.projectId, { ...wt, path, text: '' });
      refresh();
      state.setFile(made.path);
    } catch (e) { fail(e); }
  };
  const rename = async (from: string, raw: string) => {
    const to = raw.replace(/^\/+|\/+$/g, '');
    if (to === from) return;
    try {
      await api.renameFile(place.projectId, { ...wt, from, to });
      refresh();
      const shown = state.file;
      if (shown === from) state.setFile(to);
      else if (shown?.startsWith(`${from}/`)) state.setFile(to + shown.slice(from.length));
    } catch (e) { fail(e); }
  };
  const remove = async (path: string) => {
    const { ok } = await confirm({
      title: `Delete ${path.split('/').pop()}?`,
      body: <>This removes <span className="font-mono">{path}</span> from the disk. Git can bring back a committed version; uncommitted changes are lost.</>,
      action: 'Delete file',
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteFile(place.projectId, place.worktreeId, path);
      refresh();
      if (state.file === path) state.setFile(null);
    } catch (e) { fail(e); }
  };
  const entries = (e: Pick<FileEntry, 'path' | 'dir'>): MenuEntry[] => {
    const held = !!state.unsaved && (state.unsaved === e.path || state.unsaved.startsWith(`${e.path}/`));
    return [
      e.dir
        ? { input: `New file in ${e.path}…`, onSubmit: (name) => void create(`${e.path}/${name}`) }
        : { input: 'New file here…', onSubmit: (name) => void create(parentOf(e.path) ? `${parentOf(e.path)}/${name}` : name) },
      ...(held
        ? [
            { heading: 'Save or cancel your edits first' } as const,
            { label: 'Rename…', disabled: true, onSelect: () => {} },
            ...(e.dir ? [] : [{ label: 'Delete…', disabled: true, onSelect: () => {} }]),
          ]
        : [
            { input: 'Rename…', defaultValue: e.path, onSubmit: (to: string) => void rename(e.path, to) },
            ...(e.dir ? [] : [{ separator: true } as const, { label: 'Delete…', danger: true, onSelect: () => void remove(e.path) }]),
          ]),
    ];
  };
  return { create, rename, remove, entries };
}

/** The left column of the Files view: which checkout, then its files (as git sees them, `.gitignore` honoured). */
export function FolderTree({ state }: { state: FolderViewState }) {
  if (!state.place) return <EmptyState icon={<FolderOpen />}>Add a project to browse its files.</EmptyState>;
  return (
    <div className="flex flex-col gap-2 py-1">
      <div className="flex items-center gap-1 px-1">
        <div className="min-w-0 flex-1"><PlacePicker projects={state.projects} place={state.place} onPick={state.pick} /></div>
        <NewFileButton key={`${state.place.projectId}:${state.place.worktreeId}`} state={state} place={state.place} />
      </div>
      <Files key={`${state.place.projectId}:${state.place.worktreeId}`} state={state} place={state.place} />
    </div>
  );
}

/**
 * One folder of the checkout as its own screen (phones, PDT-120): its entries as rows, a folder opening its own screen
 * (its path ends in `/`) and a file its viewer. The top one (`dir` empty) has the checkout picker.
 */
export function FolderList({ state, dir }: { state: FolderViewState; dir: string }) {
  if (!state.place) return <EmptyState icon={<FolderOpen />}>Add a project to browse its files.</EmptyState>;
  return (
    <div className="flex flex-col gap-2 py-1">
      {dir === '' && <div className="px-1"><PlacePicker projects={state.projects} place={state.place} onPick={state.pick} /></div>}
      <FolderEntries key={`${state.place.projectId}:${state.place.worktreeId}`} state={state} place={state.place} dir={dir} />
    </div>
  );
}

function FolderEntries({ state, place, dir }: { state: FolderViewState; place: Place; dir: string }) {
  const q = useFolder(place, dir);
  if (q.isPending) return <Skeleton lines={6} label="Loading files" className="px-2" />;
  if (q.isError) return <div className="px-1"><Alert tone="error">{q.error.message}</Alert></div>;
  const dirs = new Set(q.data.filter((e) => e.dir).map((e) => e.path));
  return (
    <List aria-label={dir ? `Files in ${dir}` : 'Files'} onAction={(path) => state.setFile(dirs.has(path) ? `${path}/` : path)}
      renderEmpty={() => <p className="px-2 py-1 text-xs text-muted-foreground">{dir ? 'This folder is empty.' : 'This checkout has no files.'}</p>}>
      {q.data.map((e) => (
        <ListRow key={e.path} id={e.path} title={e.name} textValue={e.name}
          className={e.status === 'D' ? 'line-through text-muted-foreground' : undefined}
          leading={e.dir ? <Folder size={15} aria-hidden /> : <File size={15} aria-hidden />}
          trailing={(e.status || e.dir) ? (
            <span className="flex items-center gap-1">
              <StatusMark status={e.status} />
              {e.dir && <ChevronRight aria-hidden className="size-4 text-muted-foreground" />}
            </span>
          ) : undefined} />
      ))}
    </List>
  );
}

/**
 * The selected file; a Markdown file shows rendered or as source. Text files can be edited, except `readOnly` (phones
 * are view-only, PDT-120).
 */
export function FileViewer({ state, readOnly = false }: { state: FolderViewState; readOnly?: boolean }) {
  const place = state.place;
  const q = useQuery({
    queryKey: ['file', place?.projectId, place?.worktreeId, state.file],
    queryFn: () => api.fileContent(place!.projectId, place!.worktreeId, state.file!),
    enabled: !!place && !!state.file,
  });
  if (!state.file || !place) return <EmptyState icon={<File />}>{readOnly ? 'Pick a file to view it.' : 'Pick a file to view or edit it.'}</EmptyState>;
  if (q.isPending) return <Skeleton lines={8} label="Loading the file" className="p-4" />;
  if (q.isError) return <div className="p-4"><Alert tone="error">{q.error.message}</Alert></div>;
  return <OpenFile key={`${place.projectId}:${place.worktreeId}:${q.data.path}`} state={state} place={place} file={q.data} readOnly={readOnly} />;
}

/** An edit under way: the file as it was when editing started (or last saved), and the text now. */
interface Edit {
  base: FileContent & { text: string };
  text: string;
  /** What the editor started from; a new one (Reload) remounts it. */
  start: string;
  n: number;
}

const editOf = (f: FileContent, n = 0): Edit | null => (f.text === null ? null : { base: { ...f, text: f.text }, text: f.text, start: f.text, n });

/** Agent sessions working in the checkout (they may change the file too). */
function useAgentsAt(place: Place): Session[] {
  const sessions = useQuery({ queryKey: ['sessions', place.projectId], queryFn: () => api.listSessions(place.projectId), refetchInterval: POLL });
  return (sessions.data ?? []).filter(
    (s) => s.kind === 'agent_tui' && s.status === 'running' && s.activity === 'working' && s.worktreeId === place.worktreeId,
  );
}

function OpenFile({ state, place, file: f, readOnly }: { state: FolderViewState; place: Place; file: FileContent; readOnly: boolean }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const ops = useFileOps(state, place);
  const agents = useAgentsAt(place);
  const [markdownView, setMarkdownView] = useState<'rendered' | 'source'>('rendered');
  const [editView, setEditView] = useState<'edit' | 'preview'>('edit');
  const [edit, setEdit] = useState<Edit | null>(null);
  // Saving was refused: the file changed on disk. `disk` is what is there now (null: deleted), once fetched.
  const [conflict, setConflict] = useState<{ disk?: FileContent | null; diff: boolean } | null>(null);
  const dirty = !!edit && edit.text !== edit.base.text;
  const fileKey = ['file', place.projectId, place.worktreeId, f.path];
  const wt = place.worktreeId ? { worktreeId: place.worktreeId } : {};
  const { setUnsaved } = state;
  useEffect(() => {
    if (!dirty) return;
    setUnsaved(f.path);
    return () => setUnsaved(null);
  }, [dirty, f.path]);

  useNavigationGuard(dirty, async () => (await confirm({
    title: 'Discard unsaved changes?',
    body: <>Your changes to <span className="font-mono">{f.path}</span> are not saved.</>,
    action: 'Discard changes',
    danger: true,
  })).ok);

  const tell = async (to: Session[]) => {
    try {
      await Promise.all(to.map((s) => api.tellFilesChanged(place.projectId, { sessionId: s.id, paths: [f.path] })));
      toast.success(to.length === 1 ? `${to[0].title} will be told` : `${to.length} agents will be told`, { description: 'They get the message once they are free.' });
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const save = useMutation({
    mutationFn: ({ text, baseHash }: { text: string; baseHash: string | null }) =>
      api.saveFile(place.projectId, { ...wt, path: f.path, text, ...(baseHash ? { baseHash } : {}) }),
    onSuccess: (saved, { text }) => {
      qc.setQueryData(fileKey, saved);
      void qc.invalidateQueries({ queryKey: ['files', place.projectId, place.worktreeId] });
      setConflict(null);
      // Typing during the save keeps the newer text unsaved.
      setEdit((e) => (e && saved.text !== null ? { ...e, base: { ...saved, text: saved.text }, text: e.text === text ? saved.text : e.text } : e));
      const working = agents;
      toast.success(`Saved ${f.path.split('/').pop()}`, working.length ? {
        description: `${working.map((s) => s.title).join(', ')} ${working.length === 1 ? 'is' : 'are'} working here.`,
        action: { label: working.length === 1 ? 'Tell the agent' : 'Tell the agents', onClick: () => void tell(working) },
        duration: 15_000,
      } : undefined);
    },
    onError: (e) => {
      if (e instanceof ApiError && e.code === 'file_changed') setConflict({ diff: false });
      else toast.error(e.message);
    },
  });
  const doSave = () => { if (edit && dirty && !save.isPending) save.mutate({ text: edit.text, baseHash: edit.base.hash }); };
  // ⌘S / Ctrl+S while editing (anywhere on the page: the browser's own save is never wanted here).
  const saveRef = useRef(doSave);
  saveRef.current = doSave;
  const editing = !!edit;
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault();
        saveRef.current();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [editing]);

  /** The file as it is on disk now (null when it was deleted). */
  const fetchDisk = async (): Promise<FileContent | null> => {
    try {
      const disk = await api.fileContent(place.projectId, place.worktreeId, f.path);
      qc.setQueryData(fileKey, disk);
      return disk;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  };
  const reload = async () => {
    try {
      const disk = await fetchDisk();
      setConflict(null);
      setEdit((e) => (disk ? editOf(disk, (e?.n ?? 0) + 1) : null));
    } catch (e) { toast.error(e instanceof Error ? e.message : String(e)); }
  };
  const showDiff = async () => {
    try {
      setConflict({ disk: await fetchDisk(), diff: true });
    } catch (e) { toast.error(e instanceof Error ? e.message : String(e)); }
  };
  const overwrite = () => {
    if (!edit || conflict?.disk === undefined) return;
    save.mutate({ text: edit.text, baseHash: conflict.disk?.hash ?? null });
  };
  const cancel = async () => {
    if (dirty && !(await confirm({ title: 'Discard unsaved changes?', body: 'Your changes are not saved.', action: 'Discard changes', danger: true })).ok) return;
    setEdit(null);
    setConflict(null);
  };

  const readable = !f.tooLarge && !f.binary && f.text !== null;
  const editable = readable && !readOnly;
  const markdown = isMarkdownPath(f.path) && readable;
  const name = f.path.split('/').pop() ?? f.path;
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
            {dirty ? <span className="shrink-0 text-waiting">Unsaved changes</span> : <span className="shrink-0">{f.size.toLocaleString()} bytes</span>}
          </>
        }
        actions={
          <>
            {markdown && !edit && (
              <SegmentedControl<'rendered' | 'source'>
                aria-label="Show"
                size="sm"
                options={[{ value: 'rendered', label: 'Rendered' }, { value: 'source', label: 'Source' }]}
                value={markdownView}
                onChange={setMarkdownView}
              />
            )}
            {markdown && edit && (
              <SegmentedControl<'edit' | 'preview'>
                aria-label="Editor"
                size="sm"
                options={[{ value: 'edit', label: 'Edit' }, { value: 'preview', label: 'Preview' }]}
                value={editView}
                onChange={setEditView}
              />
            )}
            {edit ? (
              <>
                <Button size="sm" variant="secondary" onClick={() => void cancel()}>{dirty ? 'Cancel' : 'Done'}</Button>
                <Button size="sm" title="Save (⌘S)" disabled={!dirty} pending={save.isPending} onClick={doSave}>Save</Button>
              </>
            ) : (
              editable && <Button size="sm" variant="secondary" onClick={() => { setEditView('edit'); setEdit(editOf(f)); }}>Edit</Button>
            )}
            <MenuButton
              label="File actions"
              size="sm"
              placement="bottom end"
              entries={() => [
                { label: 'Copy path', onSelect: () => void copyToClipboard(f.path, 'Path copied') },
                ...(readOnly ? [] : [{ separator: true } as const, ...ops.entries({ path: f.path, dir: false })]),
              ]}
            />
          </>
        }
      />
      {edit && agents.length > 0 && (
        <Banner tone="waiting" title={agents.map((s) => s.title).join(', ')}>
          {agents.map((s) => s.title).join(', ')} {agents.length === 1 ? 'is' : 'are'} working in this checkout and may change this file too.
        </Banner>
      )}
      {conflict && (
        <div className="border-b border-border p-2">
          <Alert tone="warning" title={`${name} changed on disk since you opened it`}>
            <p>Saving was stopped so nothing is overwritten. Reload takes the file as it is now (your changes are dropped), or look at the difference and overwrite it with yours.</p>
            <div className="mt-2 flex gap-2">
              <Button size="sm" variant="secondary" onClick={() => void reload()}>Reload</Button>
              <Button size="sm" variant="secondary" onClick={() => void showDiff()}>Show diff</Button>
            </div>
          </Alert>
        </div>
      )}
      {conflict?.diff && edit && conflict.disk !== undefined && (
        <Dialog
          open
          onOpenChange={(o) => { if (!o) setConflict({ ...conflict, diff: false }); }}
          size="xl"
          title={`${name}: on disk → yours`}
          description={conflict.disk === null ? 'The file was deleted on disk. Overwrite makes it again with your text.' : 'Removed lines are on disk now; added lines are your changes.'}
          footer={
            <>
              <Button variant="secondary" onClick={() => setConflict({ ...conflict, diff: false })}>Cancel</Button>
              <Button variant="secondary" onClick={() => void reload()}>Reload</Button>
              <Button variant="danger" pending={save.isPending} onClick={overwrite}>Overwrite</Button>
            </>
          }
        >
          <div className="h-[60vh]">
            <Suspense fallback={<Skeleton lines={8} />}>
              <CodeDiff path={f.path} oldText={conflict.disk?.text ?? ''} newText={edit.text} mode="unified" wholeFile={false}
                anchors={[]} renderAnchor={() => null} onComment={() => {}} />
            </Suspense>
          </div>
        </Dialog>
      )}
      {f.tooLarge ? (
        <EmptyState icon={<FileQuestion />}>This file is over 1 MB, too large to show or edit here.</EmptyState>
      ) : f.binary || f.text === null ? (
        <EmptyState icon={<FileQuestion />}>This is a binary file.</EmptyState>
      ) : edit ? (
        <>
          {/* The editor stays mounted under the preview: it holds the text and its undo history. */}
          <div className={cn('flex-1 min-h-0', markdown && editView === 'preview' && 'hidden')}>
            {markdown ? (
              <MarkdownEditor key={edit.n} variant="file" label={f.path} value={edit.text} onChange={(text) => setEdit((e) => (e ? { ...e, text } : e))} />
            ) : (
              <CodeViewer key={edit.n} name={f.path} text={edit.start} onChange={(text) => setEdit((e) => (e ? { ...e, text } : e))} />
            )}
          </div>
          {markdown && editView === 'preview' && (
            <div className="min-h-0 flex-1 overflow-auto">
              <Markdown className="mx-auto max-w-3xl p-4">{edit.text}</Markdown>
            </div>
          )}
        </>
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
