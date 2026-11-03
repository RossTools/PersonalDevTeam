import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from '@ui/Toaster';

export interface ActionOptions {
  /** Toasted when the action succeeds ("Branch pushed"). Errors always toast. */
  success?: string;
}

/** The message to show for a failed action: an ApiError's (or any Error's) message, else a generic one. */
export function actionError(err: unknown): string {
  if (err instanceof Error && err.message) return err.message;
  if (typeof err === 'string' && err) return err;
  return 'Something went wrong';
}

/**
 * Runs an async action outside React (menu entries, drop handlers): toasts the error on failure and never rejects.
 * Resolves to the action's result, or undefined when it failed.
 */
export async function runAction<R>(fn: () => Promise<R> | R, opts?: ActionOptions): Promise<R | undefined> {
  try {
    const r = await fn();
    if (opts?.success) toast.success(opts.success);
    return r;
  } catch (err) {
    toast.error(actionError(err));
    return undefined;
  }
}

/**
 * `const [run, pending] = useAction(fn, { success })`: `run(...args)` calls `fn`, `pending` is true until it settles,
 * a rejection toasts its message and `run` resolves to undefined instead of throwing.
 */
export function useAction<A extends unknown[], R>(
  fn: (...args: A) => Promise<R> | R,
  opts?: ActionOptions,
): [run: (...args: A) => Promise<R | undefined>, pending: boolean] {
  const fnRef = useRef(fn);
  const optsRef = useRef(opts);
  useEffect(() => {
    fnRef.current = fn;
    optsRef.current = opts;
  });
  const [count, setCount] = useState(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  const run = useCallback(async (...args: A) => {
    setCount((c) => c + 1);
    try {
      return await runAction(() => fnRef.current(...args), optsRef.current);
    } finally {
      if (mounted.current) setCount((c) => c - 1);
    }
  }, []);
  return [run, count > 0];
}
