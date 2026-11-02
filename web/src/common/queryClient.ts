import { MutationCache, QueryClient, type QueryClientConfig } from '@tanstack/react-query';
import { toast } from '@ui/Toaster';
import { actionError } from './useAction';

declare module '@tanstack/react-query' {
  interface Register {
    mutationMeta: {
      /** The call site shows the mutation's error itself (inline, a save status, its own toast): no fallback toast. */
      errorShown?: boolean;
    };
  }
}

/**
 * The app's QueryClient. A failed mutation toasts its error unless it has its own `onError` or says it shows the error
 * (`meta: { errorShown: true }`), so none fails silently and none reports twice.
 */
export function createQueryClient(config: QueryClientConfig = {}): QueryClient {
  return new QueryClient({
    ...config,
    mutationCache: new MutationCache({
      onError: (err, _vars, _ctx, mutation) => {
        if (mutation.meta?.errorShown || mutation.options.onError) return;
        toast.error(actionError(err));
      },
    }),
  });
}
