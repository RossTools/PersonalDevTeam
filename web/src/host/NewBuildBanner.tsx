import { Banner, Button } from '@ui';
import { useReloadOnNewBuild } from './useReloadOnNewBuild';

/**
 * Reloads the page on a new web build (`useReloadOnNewBuild`) and, while a draft or a dialog holds the reload back,
 * offers it in a one-line strip. The strip sits in the page's flow above the screen, so on a phone it pushes the
 * composer down rather than covering it.
 */
export function NewBuildBanner({ reload }: { reload?: () => void }) {
  const build = useReloadOnNewBuild(reload);
  if (!build.held) return null;
  return (
    <Banner
      tone="accent"
      role="status"
      actions={<Button size="sm" variant="primary" compact onClick={build.reload}>Reload</Button>}
    >
      PDT was updated
    </Banner>
  );
}
