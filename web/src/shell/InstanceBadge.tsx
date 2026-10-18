import { Badge } from '@ui';
import { useInstance } from './useInstance';

/** `DEV · <port>` on a development instance (dev profile or a non-stable port); nothing on the stable one. */
export function InstanceBadge() {
  const health = useInstance().data;
  if (!health?.dev) return null;
  return (
    <Badge tone="waiting" className="font-mono" title="Development instance">
      DEV · {health.port}
    </Badge>
  );
}
