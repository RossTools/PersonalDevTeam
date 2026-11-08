// Connections: the list every picker and settings page shares.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';

export const connectionsKey = ['connections'] as const;

/** Every connection (`GET /api/connections`); read only while `enabled`. */
export const useConnections = (enabled = true) => useQuery({ queryKey: connectionsKey, queryFn: api.listConnections, enabled });

const statusKey = (id: string) => ['connection-status', id] as const;

/** How a connection is doing beyond OK or Failing, as its plugin tells it. */
export const useConnectionStatus = (id: string) =>
  useQuery({ queryKey: statusKey(id), queryFn: () => api.connectionStatus(id), refetchInterval: POLL.steady });
