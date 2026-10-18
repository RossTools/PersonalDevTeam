import { Badge } from '@ui';
import { cn } from '@ui/lib/utils';
import { useInstance } from './useInstance';

/**
 * `DEV · <port>` on a development instance (dev profile or a non-stable port); nothing on the stable one. `stacked`
 * puts DEV above the port, for the narrow desktop rail; `compact` shows just DEV (the port in its tooltip), for the phone
 * header where every pixel goes to the tabs.
 */
export function InstanceBadge({ stacked = false, compact = false }: { stacked?: boolean; compact?: boolean }) {
  const health = useInstance().data;
  if (!health?.dev) return null;
  return (
    <Badge
      tone="waiting"
      className={cn('font-mono', stacked && 'h-auto flex-col gap-0 px-1 py-0.5 leading-tight')}
      title={`Development instance on port ${health.port}`}
    >
      {stacked ? (
        <>
          <span>DEV</span>
          <span>{health.port}</span>
        </>
      ) : compact ? (
        'DEV'
      ) : (
        <>DEV · {health.port}</>
      )}
    </Badge>
  );
}
