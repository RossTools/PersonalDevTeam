// Plugins: the list the catalog, the PM, the settings search and the ticket view share.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';
import { pluginsKey } from './model';

/** Every plugin (`GET /api/integrations`). */
export const usePlugins = ({ enabled = true }: { enabled?: boolean } = {}) => useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, enabled });

export const forgeLoginKey = (kind: string) => ['forge-login', kind] as const;

/** A forge's logins (`github`), polled only while a device code waits to be entered: PDT signs in as soon as the forge
 * says so. */
export function useForgeLogin(kind: string) {
  return useQuery({
    queryKey: forgeLoginKey(kind),
    queryFn: () => api.forgeLogin(kind),
    refetchInterval: (q) => (q.state.data?.pending ? POLL.live : false),
  });
}
