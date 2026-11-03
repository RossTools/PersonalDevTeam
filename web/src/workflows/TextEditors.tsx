// The editors a node's long text fields get (PDT-250): Markdown for prompts and messages, code for scripts and JSON,
// each with an Expand button that opens the same editor in a dialog over most of the screen.
import { lazy, Suspense, useState, type ComponentProps, type ReactNode } from 'react';
import { Maximize2 } from 'lucide-react';
import { Button, Dialog, DialogActions, IconButton, Skeleton } from '@ui';
import { MarkdownEditor } from '../common/LazyMarkdownEditor';

// CodeMirror loads with the first editor shown, not with the workflow builder.
const LazyCodeEditor = lazy(() => import('./CodeEditor').then((m) => ({ default: m.CodeEditor })));

function CodeEditor(props: ComponentProps<typeof LazyCodeEditor>) {
  return (
    <Suspense fallback={<Skeleton lines={3} className="p-2" />}>
      <LazyCodeEditor {...props} />
    </Suspense>
  );
}

/** A file name CodeEditor picks the language from. */
const fileFor = (language: string) => `field.${language || 'txt'}`;

/** Edits a copy of `value` in a large dialog: Done hands it back, Cancel drops it. */
function ExpandButton({ label, value, onDone, readOnly, editor }: {
  label: string;
  value: string;
  onDone: (text: string) => void;
  readOnly?: boolean;
  editor: (text: string, onChange: (text: string) => void) => ReactNode;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const close = () => setDraft(null);
  return (
    <>
      <IconButton label={`Expand ${label}`} size="sm" onClick={() => setDraft(value)}><Maximize2 aria-hidden /></IconButton>
      <Dialog
        open={draft !== null}
        onClose={close}
        title={label}
        size="full"
        dismissable={false}
        footer={readOnly ? <Button onClick={close}>Close</Button> : (
          <DialogActions onCancel={close} action="Done" onAction={() => { onDone(draft ?? ''); close(); }} />
        )}
      >
        {draft !== null && (
          <div className="h-full overflow-hidden rounded-md border border-border">{editor(draft, setDraft)}</div>
        )}
      </Dialog>
    </>
  );
}

export function MarkdownField({ label, value, onChange, readOnly, extra }: {
  label: string; value: string; onChange: (text: string) => void; readOnly?: boolean; extra?: ReactNode;
}) {
  return (
    <div className="flex items-start gap-1">
      <div className="max-h-80 min-w-0 flex-1 overflow-auto rounded-md border border-input bg-card px-2 py-1.5 focus-within:border-primary">
        <MarkdownEditor variant="field" label={label} value={value} onChange={onChange} readOnly={readOnly} />
      </div>
      <div className="flex flex-col gap-1">
        <ExpandButton label={label} value={value} onDone={onChange} readOnly={readOnly}
          editor={(text, set) => <MarkdownEditor variant="file" label={`${label} (expanded)`} value={text} onChange={set} readOnly={readOnly} autoFocus="end" />} />
        {extra}
      </div>
    </div>
  );
}

export function CodeField({ label, language, value, onChange, readOnly, extra }: {
  label: string; language: string; value: string; onChange: (text: string) => void; readOnly?: boolean; extra?: ReactNode;
}) {
  return (
    <div className="flex items-start gap-1">
      <div className="h-40 min-w-0 flex-1 resize-y overflow-hidden rounded-md border border-input focus-within:border-primary">
        <CodeEditor name={fileFor(language)} label={label} value={value} onChange={onChange} readOnly={readOnly} />
      </div>
      <div className="flex flex-col gap-1">
        <ExpandButton label={label} value={value} onDone={onChange} readOnly={readOnly}
          editor={(text, set) => <CodeEditor name={fileFor(language)} label={`${label} (expanded)`} value={text} onChange={set} readOnly={readOnly} />} />
        {extra}
      </div>
    </div>
  );
}
