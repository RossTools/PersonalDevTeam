// The text view: the draft as JSON or TOML. A parsed edit becomes the draft; a parse error shows
// and leaves the draft as it was.
import { lazy, Suspense, useEffect, useState } from 'react';
import { Alert, SegmentedControl, Skeleton } from '@ui';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { fromText, sameDoc, toText, type TextFormat } from './textFormat';

const CodeEditor = lazy(() => import('./CodeEditor').then((m) => ({ default: m.CodeEditor })));

export function TextView({ doc, onChange, readOnly = false }: { doc: WorkflowDoc; onChange?: (doc: WorkflowDoc) => void; readOnly?: boolean }) {
  const [format, setFormat] = useState<TextFormat>('json');
  const [text, setText] = useState(() => toText(doc, format));
  const [error, setError] = useState<string | null>(null);
  // A change made in another view (or a new format) rewrites the text, unless the text already says it.
  useEffect(() => {
    setText((t) => {
      const parsed = fromText(t, format);
      return 'doc' in parsed && sameDoc(parsed.doc, doc) ? t : toText(doc, format);
    });
  }, [doc, format]);
  const edit = (t: string) => {
    setText(t);
    const parsed = fromText(t, format);
    if ('error' in parsed) return setError(parsed.error);
    setError(null);
    if (!sameDoc(parsed.doc, doc)) onChange?.(parsed.doc);
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
        <SegmentedControl aria-label="Format" size="sm" value={format} onChange={(f) => { setFormat(f); setError(null); }}
          options={[{ value: 'json', label: 'JSON' }, { value: 'toml', label: 'TOML' }]} />
        <span className="text-meta text-muted-foreground">{readOnly || !onChange ? 'Read only' : 'Edits apply as you type once the text reads.'}</span>
      </div>
      {error && <Alert tone="error" className="m-2">Not applied: {error}</Alert>}
      <div className="min-h-0 flex-1">
        <Suspense fallback={<Skeleton lines={8} className="p-3" />}>
          <CodeEditor name={`workflow.${format}`} label={`Workflow as ${format.toUpperCase()}`} value={text} onChange={edit} readOnly={readOnly || !onChange} />
        </Suspense>
      </div>
    </div>
  );
}
