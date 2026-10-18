import { Badge } from '@ui';
import { cn } from '@ui/lib/utils';
import { useInstance } from './useInstance';

/**
 * `DEV · <port>` on a development instance (dev profile or a non-stable port); nothing on the stable one. `stacked`
 * puts DEV above the port, for the narrow desktop rail.
 */
export function InstanceBadge({ stacked = false }: { stacked?: boolean }) {
  const health = useInstance().data;
  if (!health?.dev) return null;
  return (
    <Badge
      tone="waiting"
      className={cn('font-mono', stacked && 'h-auto flex-col gap-0 px-1 py-0.5 leading-tight')}
      title="Development instance"
    >
      {stacked ? (
        <>
          <span>DEV</span>
          <span>{health.port}</span>
        </>
      ) : (
        <>DEV · {health.port}</>
      )}
    </Badge>
  );
}
