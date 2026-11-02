import { lazy, Suspense, type ComponentProps } from 'react';
import { Skeleton } from '@ui';

// The Markdown libraries (react-markdown, remark, rehype) are loaded when rendered Markdown is first shown, not with the
// app: for what the shell itself shows (the update banner's release notes, the Files view).
const Lazy = lazy(() => import('./Markdown').then((m) => ({ default: m.Markdown })));

export function Markdown(props: ComponentProps<typeof Lazy>) {
  return (
    <Suspense fallback={<Skeleton lines={3} />}>
      <Lazy {...props} />
    </Suspense>
  );
}
