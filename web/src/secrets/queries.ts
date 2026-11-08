// Secrets: the list Settings → Secrets and the MCP server dialog share, where values are kept, and a secret's uses.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';

export const secretKeys = {
  /** The list, and under it where values are kept: a write refreshes both. */
  all: ['secrets'] as const,
  store: ['secrets', 'store'] as const,
  uses: (id: string) => ['secret-uses', id] as const,
};

/** Every secret (names and who may use them, never values). */
export const useSecrets = () => useQuery({ queryKey: secretKeys.all, queryFn: api.listSecrets });
