import { lazy, Suspense, type ComponentProps } from 'react';
import { Skeleton } from '@ui';

// CodeMirror is loaded when an editor first opens, not with the app.
const Editor = lazy(() => import('./MarkdownEditor').then((m) => ({ default: m.MarkdownEditor })));

export type { MarkdownEditorHandle } from './MarkdownEditor';

export function MarkdownEditor(props: ComponentProps<typeof Editor>) {
  return (
    <Suspense fallback={<Skeleton lines={6} />}>
      <Editor {...props} />
    </Suspense>
  );
}
