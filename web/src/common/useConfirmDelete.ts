import { useMutation, useQueryClient, type QueryKey } from '@tanstack/react-query';
import { useConfirm, type ConfirmOptions } from '@ui';

/**
 * Removing an item from a settings list (a secret, a server, a connection): `remove.ask(item)` confirms with
 * `confirm(item)` (a danger action), then deletes it and invalidates `invalidate`. `remove.error` is the last failure,
 * for an Alert under the list.
 */
export function useConfirmDelete<T>({ remove, confirm: options, invalidate }: {
  remove: (item: T) => Promise<unknown>;
  confirm: (item: T) => Omit<ConfirmOptions, 'danger'>;
  invalidate: QueryKey[];
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const m = useMutation({
    meta: { errorShown: true },
    mutationFn: remove,
    onSuccess: () => {
      for (const queryKey of invalidate) void qc.invalidateQueries({ queryKey });
    },
  });
  const ask = async (item: T) => {
    const { ok } = await confirm({ ...options(item), danger: true });
    if (ok) m.mutate(item);
  };
  return { ask, error: m.error, pending: m.isPending };
}
