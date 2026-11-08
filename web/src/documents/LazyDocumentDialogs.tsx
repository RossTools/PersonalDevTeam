import { lazy, Suspense, type ComponentProps } from 'react';

// The document dialogs (new, import, link) are loaded the first time one opens, not with the app.
const New = lazy(() => import('./DocumentDialogs').then((m) => ({ default: m.NewDocumentDialog })));
const Import = lazy(() => import('./DocumentDialogs').then((m) => ({ default: m.ImportDocumentDialog })));
const Link = lazy(() => import('./DocumentDialogs').then((m) => ({ default: m.LinkDocumentDialog })));

export function NewDocumentDialog(props: ComponentProps<typeof New>) {
  return <Suspense fallback={null}><New {...props} /></Suspense>;
}

export function ImportDocumentDialog(props: ComponentProps<typeof Import>) {
  return <Suspense fallback={null}><Import {...props} /></Suspense>;
}

export function LinkDocumentDialog(props: ComponentProps<typeof Link>) {
  return <Suspense fallback={null}><Link {...props} /></Suspense>;
}
