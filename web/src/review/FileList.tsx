import { useMemo, useState } from 'react';
import { Check, File, Folder, MessageSquare } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { DiffStat, TreeView, type TreeChildren, type TreeNode } from '@ui';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { ChangeStatus } from '../api/gen/ChangeStatus';

const LETTER: Record<ChangeStatus, string> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R' };
const LETTER_TONE: Record<ChangeStatus, string> = {
  added: 'text-running', modified: 'text-waiting', deleted: 'text-failed', renamed: 'text-primary',
};

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

/** The changed files, one folder level per directory path (as GitHub groups them): status, +/−, comments, viewed. */
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
        trailing: (
          <span className="flex items-center gap-1.5 text-meta">
            {n > 0 && (
              <span className="flex items-center gap-0.5 text-muted-foreground" title={`${n} comment thread${n === 1 ? '' : 's'}`}>
                <MessageSquare aria-hidden className="size-3" />
                {n}
              </span>
            )}
            {seen && <Check aria-label="Viewed" className="size-3 text-muted-foreground" />}
            {!f.binary && <DiffStat added={f.added} removed={f.removed} />}
            <span className={cn('w-3 text-center font-mono font-semibold', LETTER_TONE[f.status])} title={f.status}>
              {LETTER[f.status]}
            </span>
          </span>
        ),
      };
    };
    const byDir = new Map<string, ChangedFile[]>();
    for (const f of files) {
      const d = dirOf(f.path);
      byDir.set(d, [...(byDir.get(d) ?? []), f]);
    }
    const dirs = [...byDir.keys()].filter((d) => d !== '').sort();
    const top: Node[] = dirs.map((d) => ({
      id: `dir:${d}`,
      label: `${d}/`,
      hasChildren: true,
      icon: <Folder aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />,
      children: (byDir.get(d) ?? []).map(leaf),
    }));
    return [...top, ...(byDir.get('') ?? []).map(leaf)];
  }, [files, comments, viewed]);
  // Folders start open; the ones the user closes stay closed.
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const dirIds = items.filter((n) => n.hasChildren).map((n) => n.id);
  const expanded = dirIds.filter((id) => !closed.has(id));
  const useChildren = (n: Node): TreeChildren<Node> => ({ status: 'ready', items: n.children ?? [] });
  return (
    <TreeView<Node>
      aria-label="Changed files"
      items={items}
      useChildren={useChildren}
      expandedKeys={expanded}
      onExpandedChange={(open) => setClosed(new Set(dirIds.filter((id) => !open.has(id))))}
      selectedKey={selected}
      onAction={(n) => n.file && onSelect(n.file.path)}
      renderEmpty={() => <p className="p-3 text-xs text-muted-foreground">No changes yet.</p>}
      className={className}
    />
  );
}
