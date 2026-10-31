import { cn } from './lib/utils';

export type DiffLineKind = 'add' | 'del' | 'ctx';

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  /** Line number before the change (`del`, `ctx`), when the hunk says. */
  old?: number;
  /** Line number after the change (`add`, `ctx`), when the hunk says. */
  new?: number;
}

export interface DiffHunk {
  /** The `@@ … @@` line. */
  header: string;
  lines: DiffLine[];
}

export interface DiffFile {
  /** The file's path after the change (before it, for a deleted file), without git's `a/` / `b/`. */
  path: string;
  created: boolean;
  deleted: boolean;
  hunks: DiffHunk[];
  added: number;
  removed: number;
}

const stripSide = (p: string) => p.replace(/^[ab]\//, '').replace(/\t.*$/, '');

/** Reads a unified diff (one or more files; `@@ @@` hunks without line numbers are fine). */
export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldNo = 0;
  let newNo = 0;
  let numbered = false;
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith('--- ') && lines[i + 1]?.startsWith('+++ ')) {
      const from = line.slice(4).trim();
      const to = lines[i + 1].slice(4).trim();
      file = {
        path: stripSide(to === '/dev/null' ? from : to),
        created: from === '/dev/null',
        deleted: to === '/dev/null',
        hunks: [],
        added: 0,
        removed: 0,
      };
      files.push(file);
      hunk = null;
      i++;
      continue;
    }
    if (line.startsWith('diff ') || line.startsWith('index ')) continue;
    if (line.startsWith('@@')) {
      if (!file) {
        file = { path: '', created: false, deleted: false, hunks: [], added: 0, removed: 0 };
        files.push(file);
      }
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
      numbered = !!m;
      oldNo = m ? Number(m[1]) : 0;
      newNo = m ? Number(m[2]) : 0;
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      continue;
    }
    if (!hunk || !file) continue;
    if (line.startsWith('\\')) continue; // "\ No newline at end of file"
    const sign = line[0];
    const body = line.slice(1);
    if (sign === '+') {
      hunk.lines.push({ kind: 'add', text: body, ...(numbered ? { new: newNo++ } : {}) });
      file.added++;
    } else if (sign === '-') {
      hunk.lines.push({ kind: 'del', text: body, ...(numbered ? { old: oldNo++ } : {}) });
      file.removed++;
    } else if (sign === ' ' || (line === '' && i < lines.length - 1)) {
      hunk.lines.push({ kind: 'ctx', text: body, ...(numbered ? { old: oldNo++, new: newNo++ } : {}) });
    }
  }
  return files;
}

/** Lines added and removed across a diff. */
export function diffStats(text: string): { added: number; removed: number } {
  return parseDiff(text).reduce((s, f) => ({ added: s.added + f.added, removed: s.removed + f.removed }), { added: 0, removed: 0 });
}

/** "+12 −3" for a diff (the minus is a real minus sign). */
export function DiffStat({ added, removed, className }: { added: number; removed: number; className?: string }) {
  return (
    <span className={cn('inline-flex gap-1 font-mono tabular-nums', className)}>
      <span className="text-running">+{added}</span>
      <span className="text-failed">−{removed}</span>
    </span>
  );
}

const SIGN: Record<DiffLineKind, string> = { add: '+', del: '-', ctx: ' ' };
const ROW: Record<DiffLineKind, string> = { add: 'bg-diff-add', del: 'bg-diff-del', ctx: '' };
/** Line-number cells and signs, as GitHub tints them. */
const NUM: Record<DiffLineKind, string> = { add: 'bg-diff-add-gutter text-foreground', del: 'bg-diff-del-gutter text-foreground', ctx: 'text-muted-foreground' };
const SIGN_TONE: Record<DiffLineKind, string> = { add: 'text-diff-add-sign', del: 'text-diff-del-sign', ctx: 'text-muted-foreground' };

export interface DiffViewerProps {
  /** A unified diff: `--- a/x` / `+++ b/x` file headers, then hunks. */
  diff: string;
  /** Leave out the file header (a single file already named around it, e.g. in a tool row). */
  hideHeaders?: boolean;
  /** Wrap long lines instead of scrolling them sideways (a JSON document's long strings, PDT-320). */
  wrap?: boolean;
  className?: string;
}

/**
 * A read-only unified diff: per-file headers with "+N −M", hunks with old/new line-number gutters and add/remove
 * shading. Long lines scroll sideways within their file, or wrap with `wrap`; either way the viewer never grows wider
 * than its container. (Phase 6 adds split view and comments.)
 */
export function DiffViewer({ diff, hideHeaders = false, wrap = false, className }: DiffViewerProps) {
  const files = parseDiff(diff);
  if (files.length === 0) return <p className={cn('text-xs text-muted-foreground', className)}>No changes.</p>;
  return (
    <div className={cn('flex min-w-0 flex-col gap-2', className)}>
      {files.map((f, fi) => (
        <section key={`${f.path}:${fi}`} aria-label={f.path || 'Diff'} className="overflow-hidden rounded-md border border-border bg-card">
          {!hideHeaders && f.path && (
            <header className="flex items-center gap-2 border-b border-border bg-surface-2 px-2 py-1 font-mono text-xs">
              <span className="min-w-0 truncate" title={f.path}>{f.path}</span>
              {f.created && <span className="shrink-0 text-2xs text-muted-foreground">new</span>}
              {f.deleted && <span className="shrink-0 text-2xs text-muted-foreground">deleted</span>}
              <DiffStat added={f.added} removed={f.removed} className="ml-auto shrink-0" />
            </header>
          )}
          <div className="overflow-x-auto">
            <table className="w-full border-collapse font-mono text-xs leading-5">
              <tbody>
                {f.hunks.map((h, hi) => [
                  <tr key={`h${hi}`} className="bg-diff-hunk text-diff-hunk-fg">
                    <td colSpan={3} className="px-2 select-none">{/\d/.test(h.header) ? h.header : '⋯'}</td>
                  </tr>,
                  ...h.lines.map((l, li) => (
                    <tr key={`${hi}:${li}`} data-kind={l.kind} className={ROW[l.kind]}>
                      <td className={cn('w-px px-1.5 text-right whitespace-nowrap tabular-nums select-none', NUM[l.kind])}>{l.old ?? ''}</td>
                      <td className={cn('w-px px-1.5 text-right whitespace-nowrap tabular-nums select-none', NUM[l.kind])}>{l.new ?? ''}</td>
                      <td className={cn('pr-2', wrap ? 'whitespace-pre-wrap wrap-anywhere' : 'whitespace-pre')}>
                        <span aria-hidden className={cn('inline-block w-3 select-none', SIGN_TONE[l.kind])}>{SIGN[l.kind]}</span>
                        <span className="sr-only">{l.kind === 'add' ? 'Added: ' : l.kind === 'del' ? 'Removed: ' : ''}</span>
                        {l.text}
                      </td>
                    </tr>
                  )),
                ])}
              </tbody>
            </table>
          </div>
        </section>
      ))}
    </div>
  );
}
