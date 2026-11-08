import { lazy, Suspense, type ComponentProps } from 'react';

// The due-date picker and its calendar are loaded the first time a picker opens, not with the app.
const Picker = lazy(() => import('./DuePicker').then((m) => ({ default: m.DuePicker })));

export function DuePicker(props: ComponentProps<typeof Picker>) {
  return (
    <Suspense fallback={null}>
      <Picker {...props} />
    </Suspense>
  );
}
