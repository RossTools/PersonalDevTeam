import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, File, Folder, MessageSquare } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { DiffStat, EmptyState, TreeView, type TreeChildren, type TreeNode } from '@ui';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { ChangeStatus } from '../api/gen/ChangeStatus';
import { GIT_STATUS_TONE } from '../common/gitStatus';
import { plural } from '../common/text';

const LETTER: Record<ChangeStatus, keyof typeof GIT_STATUS_TONE> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R' };

interface Node extends TreeNode {
  file?: ChangedFile;
  children?: Node[];
}

const dirOf = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '');
const nameOf = (p: string) => p.slice(p.lastIndexOf('/') + 1);

export interface FileListProps {
  files: ChangedFile[];
  selected: string | null;
  onSelect: (path: string) => void;
  /** Comment threads per path. */
  comments: Record<string, number>;
  viewed: (f: ChangedFile) => boolean;
  className?: string;
}

/** A file's comment threads, viewed tick, +/− and status letter. */
function FileMarks({ file: f, comments: n, seen }: { file: ChangedFile; comments: number; seen: boolean }) {
  return (
    <span className="flex items-center gap-1.5 text-meta">
      {n > 0 && (
        <span className="flex items-center gap-0.5 text-muted-foreground" title={plural(n, 'comment thread')}>
          <MessageSquare aria-hidden className="size-3" />
          {n}
        </span>
      )}
      {seen && <Check aria-label="Viewed" className="size-3 text-muted-foreground" />}
      {!f.binary && <DiffStat added={f.added} removed={f.removed} />}
      <span className={cn('w-3 text-center font-mono font-semibold', GIT_STATUS_TONE[LETTER[f.status]])} title={f.status}>
        {LETTER[f.status]}
      </span>
    </span>
  );
}

/** The files by folder: each folder's files (folders in path order), then the top-level ones. */
function byFolder(files: ChangedFile[]): [string, ChangedFile[]][] {
  const byDir = new Map<string, ChangedFile[]>();
  for (const f of files) {
    const d = dirOf(f.path);
    byDir.set(d, [...(byDir.get(d) ?? []), f]);
  }
  const dirs = [...byDir.keys()].filter((d) => d !== '').sort();
  return [...dirs, ...(byDir.has('') ? [''] : [])].map((d) => [d, byDir.get(d)!]);
}

/** The files in the order the list shows them (the Changes page shows them in the same order). */
export const inListOrder = (files: ChangedFile[]): ChangedFile[] => byFolder(files).flatMap(([, fs]) => fs);

/**
 * The changed files, one folder level per directory path (as GitHub groups them): status, +/−, comments, viewed. The
 * Changes page's navigator: `selected` is the file in view, kept in sight as the page scrolls.
 */
export function FileList({ files, selected, onSelect, comments, viewed, className }: FileListProps) {
  const items = useMemo(() => {
    const leaf = (f: ChangedFile): Node => {
      const seen = viewed(f);
      const n = comments[f.path] ?? 0;
      return {
        id: f.path,
        label: nameOf(f.path),
        title: f.oldPath ? `${f.oldPath} → ${f.path}` : f.path,
        file: f,
        icon: <File aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />,
        labelClassName: cn(f.status === 'deleted' && 'line-through', seen && 'text-muted-foreground'),
        trailing: <FileMarks file={f} comments={n} seen={seen} />,
      };
    };
    return byFolder(files).flatMap(([d, fs]): Node[] => (d === '' ? fs.map(leaf) : [{
      id: `dir:${d}`,
      label: `${d}/`,
      hasChildren: true,
      icon: <Folder aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />,
      children: fs.map(leaf),
    }]));
  }, [files, comments, viewed]);
  // Folders start open; the ones the user closes stay closed.
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const dirIds = items.filter((n) => n.hasChildren).map((n) => n.id);
  const expanded = dirIds.filter((id) => !closed.has(id));
  const useChildren = (n: Node): TreeChildren<Node> => ({ status: 'ready', items: n.children ?? [] });
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!selected) return;
    // A frame later: the tree draws its rows after its first render.
    const id = requestAnimationFrame(() => {
      const row = [...(box.current?.querySelectorAll<HTMLElement>('[role="row"][data-key]') ?? [])].find((r) => r.dataset.key === selected);
      row?.scrollIntoView?.({ block: 'nearest' });
    });
    return () => cancelAnimationFrame(id);
  }, [selected]);
  return (
    // The tree only reports a change of selection: a click on the file already in view scrolls back to its top too.
    <div
      ref={box}
      className="flex min-h-0 flex-1 flex-col"
      onClick={(e) => {
        const key = (e.target as HTMLElement).closest<HTMLElement>('[role="row"][data-key]')?.dataset.key;
        if (key && key === selected) onSelect(key);
      }}
    >
    <TreeView<Node>
      aria-label="Changed files"
      items={items}
      useChildren={useChildren}
      expandedKeys={expanded}
      onExpandedChange={(open) => setClosed(new Set(dirIds.filter((id) => !open.has(id))))}
      selectedKey={selected}
      onAction={(n) => n.file && onSelect(n.file.path)}
      renderEmpty={() => <EmptyState size="inline" className="p-3">No changes yet.</EmptyState>}
      className={className}
    />
    </div>
  );
}
