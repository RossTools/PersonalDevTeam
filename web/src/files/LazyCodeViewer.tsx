import { lazy, Suspense, type ComponentProps } from 'react';
import { Skeleton } from '@ui';

// CodeMirror and its language data are loaded only when a file or a config is first shown, not with the app.
const Viewer = lazy(() => import('./CodeViewer').then((m) => ({ default: m.CodeViewer })));

export function CodeViewer(props: ComponentProps<typeof Viewer>) {
  return (
    <Suspense fallback={<Skeleton lines={6} className="p-3" />}>
      <Viewer {...props} />
    </Suspense>
  );
}
