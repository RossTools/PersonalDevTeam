import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { ArrowUp, ChevronRight, Folder, FolderGit2, FolderPlus } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Alert } from './Alert';
import { Badge } from './Badge';
import { Button } from './Button';
import { Checkbox } from './Checkbox';
import { IconButton } from './IconButton';
import { Input } from './Input';
import { List, ListRow } from './List';
import { Spinner } from './Spinner';

export interface FolderPickerFolder {
  name: string;
  /** Absolute, in the host's form. */
  path: string;
  /** With the home folder written `~`. */
  display: string;
  hidden?: boolean;
  /** A git checkout. */
  git?: boolean;
  /** The project already registered for this folder. */
  project?: { name: string; key: string } | null;
}

export interface FolderPickerLink {
  name: string;
  path: string;
}

/** One folder's subfolders, as the host reports them (the API's `FolderListing`). */
export interface FolderPickerListing {
  path: string;
  display: string;
  parent: string | null;
  /** The host's separator: `/`, or `\` on Windows (where `/` is accepted too). */
  sep: string;
  crumbs: FolderPickerLink[];
  places: FolderPickerLink[];
  folders: FolderPickerFolder[];
  truncated?: boolean;
}

/** What Choose would pick: the folder being browsed, or a subfolder whose name has been typed in full. */
export interface FolderPickerSelection {
  path: string;
  display: string;
  name: string;
}

export interface FolderPickerProps {
  /** The folder on screen (the last one loaded). */
  listing?: FolderPickerListing;
  /** A folder is being loaded. */
  loading?: boolean;
  /** Why the last folder asked for could not be listed. */
  error?: string | null;
  /** List this folder: a path from the listing, or text typed into the path field (`~/wo…`). */
  onBrowse: (path: string) => void;
  showHidden: boolean;
  onShowHiddenChange: (show: boolean) => void;
  /** Makes a folder inside the one on screen; the picker then opens it. Rejects with the reason it failed. */
  onCreateFolder?: (name: string) => Promise<FolderPickerFolder>;
  /** What Choose would pick now (null while loading or after an error). */
  onSelectionChange?: (s: FolderPickerSelection | null) => void;
  /** Focus the path field on mount (not on phones: it would raise the keyboard). */
  autoFocus?: boolean;
  className?: string;
}

const lower = (s: string) => s.toLowerCase();

/** `text` split at its last separator: the folder part (with the separator) and the name being typed. */
export function splitTyped(text: string, sep: string): { dir: string; leaf: string } {
  const isSep = (c: string) => c === '/' || (sep === '\\' && c === '\\');
  for (let i = text.length - 1; i >= 0; i--) {
    if (isSep(text[i])) return { dir: text.slice(0, i + 1), leaf: text.slice(i + 1) };
  }
  return { dir: '', leaf: text };
}

const withSep = (p: string, sep: string) => (p.endsWith('/') || p.endsWith(sep) ? p : p + sep);

function lastName(path: string, sep: string): string {
  const parts = path.split(sep === '\\' ? /[\\/]/ : '/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function commonPrefix(names: string[]): string {
  if (names.length === 0) return '';
  let p = names[0];
  for (const n of names.slice(1)) {
    let i = 0;
    while (i < p.length && i < n.length && lower(p[i]) === lower(n[i])) i++;
    p = p.slice(0, i);
  }
  return p;
}

/**
 * Browses folders on the host: breadcrumb, places (home, Projects folder, drives), the subfolders (git checkouts and
 * projects marked), a path field with type-ahead (typing filters the list; Tab completes; a separator opens the
 * folder) and "New folder". Paths are shown as the host writes them. Clicking a folder opens it; the caller's Choose
 * button takes `onSelectionChange`'s value.
 */
export function FolderPicker({
  listing,
  loading = false,
  error,
  onBrowse,
  showHidden,
  onShowHiddenChange,
  onCreateFolder,
  onSelectionChange,
  autoFocus = false,
  className,
}: FolderPickerProps) {
  const sep = listing?.sep ?? '/';
  const [text, setText] = useState('');
  // The field follows the listing after a click (not while typing, which would eat the name being typed).
  const followListing = useRef(true);
  const lastBrowsed = useRef<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!listing || loading || !followListing.current) return;
    followListing.current = false;
    const t = withSep(listing.display, listing.sep);
    lastBrowsed.current = splitTyped(t, listing.sep).dir;
    setText(t);
  }, [listing, loading]);

  const { dir, leaf } = splitTyped(text, sep);
  // Typing a separator opens that folder (after a short pause, so fast typing doesn't list every step).
  useEffect(() => {
    if (!dir || dir === lastBrowsed.current) return;
    const t = setTimeout(() => {
      lastBrowsed.current = dir;
      onBrowse(dir);
    }, 150);
    return () => clearTimeout(t);
  }, [dir, onBrowse]);

  const folders = listing?.folders ?? [];
  const shown = leaf ? folders.filter((f) => lower(f.name).startsWith(lower(leaf))) : folders;
  const exact = leaf ? folders.find((f) => lower(f.name) === lower(leaf)) : undefined;
  const selection: FolderPickerSelection | null =
    loading || error || !listing
      ? null
      : exact
        ? { path: exact.path, display: exact.display, name: exact.name }
        : { path: listing.path, display: listing.display, name: lastName(listing.path, listing.sep) };
  const selKey = selection?.path ?? null;
  const report = useRef(onSelectionChange);
  report.current = onSelectionChange;
  useEffect(() => {
    report.current?.(selection);
    // Reported when what Choose would pick changes, not on every render.
  }, [selKey, selection?.display]);

  const go = (path: string) => {
    if (listing && path === listing.path) {
      setText(withSep(listing.display, listing.sep));
      lastBrowsed.current = withSep(listing.display, listing.sep);
      return;
    }
    followListing.current = true;
    onBrowse(path);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      if (text.trim()) go(exact ? exact.path : text.trim());
    } else if (e.key === 'Tab' && !e.shiftKey && leaf && shown.length > 0 && !exact) {
      const prefix = shown.length === 1 ? shown[0].name + sep : commonPrefix(shown.map((f) => f.name));
      if (prefix.length > leaf.length) {
        e.preventDefault();
        setText(dir + prefix);
      }
    } else if (e.key === 'ArrowDown' && shown.length > 0) {
      e.preventDefault();
      listRef.current?.querySelector<HTMLElement>('[role="row"]')?.focus();
    }
  };

  return (
    <div className={cn('flex min-h-0 flex-col gap-2', className)}>
      <div className="flex min-w-0 items-center gap-1">
        <IconButton label="Up one folder" size="sm" disabled={!listing?.parent} onClick={() => listing?.parent && go(listing.parent)}>
          <ArrowUp />
        </IconButton>
        <nav aria-label="Folder path" className="min-w-0 flex-1 overflow-x-auto">
          <ol className="flex items-center whitespace-nowrap">
            {(listing?.crumbs ?? []).map((c, i, all) => (
              <li key={c.path} className="flex items-center">
                {i > 0 && <ChevronRight className="size-3 shrink-0 text-muted-foreground" aria-hidden />}
                <Button
                  size="sm"
                  variant="ghost"
                  className="px-1.5 font-mono"
                  aria-current={i === all.length - 1 ? 'location' : undefined}
                  onClick={() => go(c.path)}
                >
                  {c.name}
                </Button>
              </li>
            ))}
          </ol>
        </nav>
        {loading && <Spinner label="Loading folders" />}
      </div>
      <Input
        aria-label="Folder path"
        mono
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        autoFocus={autoFocus}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
      />
      {listing && listing.places.length > 0 && (
        <div role="group" aria-label="Places" className="flex flex-wrap gap-1">
          {listing.places.map((p) => (
            <Button key={`${p.name}:${p.path}`} size="sm" variant="secondary" title={p.path} onClick={() => go(p.path)}>
              {p.name}
            </Button>
          ))}
        </div>
      )}
      {error && <Alert tone="error">{error}</Alert>}
      <div ref={listRef} className="h-72 min-h-40 overflow-auto rounded-md border border-border p-1">
        <List
          aria-label="Folders"
          items={shown.map((f) => ({ ...f, id: f.path }))}
          dependencies={[sep]}
          onAction={(id) => go(id)}
          renderEmpty={() => (
            <p className="px-2 py-6 text-center text-xs text-muted-foreground">
              {!listing ? 'Loading…' : leaf ? `No folder here starts with “${leaf}”.` : 'No folders here.'}
            </p>
          )}
        >
          {(f) => (
            <ListRow
              id={f.id}
              title={f.name}
              textValue={f.name}
              className={f.hidden ? 'text-muted-foreground' : undefined}
              leading={f.git ? <FolderGit2 className="size-4 text-muted-foreground" aria-label="git repository" /> : <Folder className="size-4 text-muted-foreground" aria-hidden />}
              trailing={
                f.project ? (
                  <Badge title={`Already a project: ${f.project.name}`}>{f.project.key}</Badge>
                ) : f.git ? (
                  <Badge>git</Badge>
                ) : undefined
              }
            />
          )}
        </List>
        {listing?.truncated && (
          <p className="px-2 py-1 text-meta text-muted-foreground">Only the first folders are shown; type a name to go straight to it.</p>
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Checkbox label="Show hidden folders" checked={showHidden} onChange={onShowHiddenChange} />
        {onCreateFolder && listing && <NewFolder key={listing.path} create={onCreateFolder} onCreated={(f) => go(f.path)} />}
      </div>
    </div>
  );
}

function NewFolder({ create, onCreated }: { create: (name: string) => Promise<FolderPickerFolder>; onCreated: (f: FolderPickerFolder) => void }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!open) {
    return (
      <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
        <FolderPlus /> New folder
      </Button>
    );
  }
  // Not a <form>: the picker usually sits inside one (Add project), and forms can't nest.
  const submit = () => {
    if (!name.trim() || pending) return;
    setPending(true);
    setError(null);
    create(name.trim()).then(
      (f) => {
        setPending(false);
        setOpen(false);
        setName('');
        onCreated(f);
      },
      (err: unknown) => {
        setPending(false);
        setError(err instanceof Error ? err.message : String(err));
      },
    );
  };
  return (
    <div className="flex w-full flex-col gap-1.5 sm:w-auto">
      <div className="flex items-center gap-1.5">
        <Input
          aria-label="New folder name"
          size="sm"
          autoFocus
          className="min-w-0 flex-1 sm:w-48"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === 'Enter') {
              e.preventDefault();
              submit();
            }
            if (e.key === 'Escape') {
              e.stopPropagation();
              setOpen(false);
            }
          }}
        />
        <Button size="sm" variant="primary" pending={pending} disabled={!name.trim()} onClick={() => submit()}>
          Create
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
      {error && <p role="alert" className="text-meta text-destructive">{error}</p>}
    </div>
  );
}
