// Plugins: the list the catalog, the PM, the settings search and the ticket view share.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { pluginsKey } from './model';

/** Every plugin (`GET /api/integrations`). */
export const usePlugins = ({ enabled = true }: { enabled?: boolean } = {}) => useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, enabled });
