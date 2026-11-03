// Connections: the list every picker and settings page shares.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';

export const connectionsKey = ['connections'] as const;

/** Every connection (`GET /api/connections`). */
export const useConnections = () => useQuery({ queryKey: connectionsKey, queryFn: api.listConnections });
