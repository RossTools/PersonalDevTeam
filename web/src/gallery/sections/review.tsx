import { lazy, Suspense, useState, type ReactNode } from 'react';
import { CommentComposer, CommentThread, type ThreadComment } from '@ui/Comment';
import { SplitPane } from '@ui/SplitPane';
import { toast } from '@ui/Toaster';
import { Markdown } from '../../common/Markdown';
import type { LineRange } from '../../review/CodeDiff';
import { FileList } from '../../review/FileList';
import { ReviewBar } from '../../review/ReviewBar';
import type { ChangedFile } from '../../api/gen/ChangedFile';

const CodeDiff = lazy(() => import('../../review/CodeDiff').then((m) => ({ default: m.CodeDiff })));

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      {children}
    </div>
  );
}

const FILES: ChangedFile[] = [
  { path: 'README.md', oldPath: null, status: 'modified', untracked: false, binary: false, added: 4, removed: 1, version: '1' },
  { path: 'crates/pdt-git/src/review.rs', oldPath: null, status: 'added', untracked: true, binary: false, added: 310, removed: 0, version: '2' },
  { path: 'crates/pdt-git/src/lib.rs', oldPath: null, status: 'modified', untracked: false, binary: false, added: 12, removed: 3, version: '3' },
  { path: 'web/src/review/old.ts', oldPath: null, status: 'deleted', untracked: false, binary: false, added: 0, removed: 40, version: '4' },
  { path: 'web/src/review/Moved.tsx', oldPath: 'web/src/Moved.tsx', status: 'renamed', untracked: false, binary: false, added: 2, removed: 2, version: '5' },
];

const OLD = `fn main() {
    let name = "world";
    println!("Hello, {}!", name);
    // nothing else yet
}
`;
const NEW = `fn main() {
    let name = std::env::args().nth(1).unwrap_or("world".into());
    println!("Hello, {name}!");
    // nothing else yet
    greet_again(&name);
}
`;

const thread: ThreadComment[] = [
  { id: 'a', author: 'You', time: '2 h ago', body: 'Should this fall back to `world` silently?', draft: false },
  { id: 'b', author: 'You', time: 'now', body: 'Maybe print a hint when no name is given.', draft: true },
];

function DiffDemo({ mode }: { mode: 'split' | 'unified' }) {
  const [composer, setComposer] = useState<LineRange | null>(null);
  return (
    <div className="h-72 overflow-auto rounded-md border border-border">
      <Suspense fallback={null}>
        <CodeDiff
          path="main.rs"
          oldText={OLD}
          newText={NEW}
          mode={mode}
          wholeFile
          anchors={[{ id: 't', side: 'new', line: 2 }, ...(composer ? [{ id: 'c', side: composer.side, line: composer.endLine }] : [])]}
          renderAnchor={(id) =>
            id === 'c' ? (
              <CommentComposer label="Comment" action="Comment" autoFocus onSubmit={() => setComposer(null)} onCancel={() => setComposer(null)} />
            ) : (
              <CommentThread comments={thread} renderBody={(t) => <Markdown>{t}</Markdown>} onReply={() => {}} onResolve={() => {}} />
            )}
          onComment={setComposer}
        />
      </Suspense>
    </div>
  );
}

function render() {
  return (
    <div className="flex flex-col gap-6">
      <Demo label="ReviewBar · drafts, Approve, Merge…">
        <div className="rounded-md border border-border">
          <ReviewBar drafts={3} onSend={() => toast.success('Send')} sending={false} attemptActive approved={false} onApprove={() => {}} onMerge={() => {}} />
          <ReviewBar drafts={0} onSend={() => {}} sending={false} attemptActive approved onApprove={() => {}} onMerge={() => {}} />
        </div>
      </Demo>
      <Demo label="SplitPane · FileList (status, +/−, comments, viewed) and a diff (+ in the gutter, c on a selection)">
        <div className="flex h-80 rounded-md border border-border">
          <SplitPane
            id="gallery-review"
            start={<FileList files={FILES} selected="README.md" onSelect={() => {}} comments={{ 'README.md': 2 }} viewed={(f) => f.path.endsWith('lib.rs')} className="overflow-auto" />}
            end={<div className="p-3 text-xs text-muted-foreground">The selected file's diff goes here.</div>}
          />
        </div>
      </Demo>
      <div className="grid gap-6 xl:grid-cols-2">
        <Demo label="CodeDiff · split"><DiffDemo mode="split" /></Demo>
        <Demo label="CodeDiff · unified"><DiffDemo mode="unified" /></Demo>
      </div>
      <div className="grid gap-6 lg:grid-cols-2">
        <Demo label="CommentThread · sent comment, draft reply">
          <CommentThread comments={thread} title="src/main.rs:2" renderBody={(t) => <Markdown>{t}</Markdown>} onReply={() => {}} onResolve={() => {}} onEdit={() => {}} onDelete={() => {}} />
        </Demo>
        <Demo label="CommentThread · resolved (collapsed)">
          <CommentThread comments={thread} title="src/main.rs:2" resolved onResolve={() => {}} />
        </Demo>
      </div>
    </div>
  );
}

export const section = { title: 'Review', order: 80, render };
