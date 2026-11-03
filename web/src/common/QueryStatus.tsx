import type { ReactNode } from 'react';
import type { UseQueryResult } from '@tanstack/react-query';
import { Alert, Button, Skeleton } from '@ui';
import { actionError } from './useAction';

/** "Loading secrets" from "Secrets"; "MCP servers" keeps its capitals. */
function loadingLabel(what: string) {
  const lower = /^[A-Z][a-z]/.test(what) ? what[0].toLowerCase() + what.slice(1) : what;
  return `Loading ${lower}`;
}

/**
 * "<What> could not be loaded: <why>", as an error Alert, with a Retry button when `onRetry` is given. Renders nothing
 * without an error, so it can sit unconditionally above what the query fills in.
 */
export function LoadError({ what, error, onRetry, className }: {
  /** What failed to load, as the start of a sentence: "Secrets", "The run". */
  what: string;
  error: unknown;
  onRetry?: () => void;
  className?: string;
}) {
  if (!error) return null;
  return (
    <Alert tone="error" className={className} action={onRetry && <Button size="sm" onClick={onRetry}>Retry</Button>}>
      {what} could not be loaded: {actionError(error)}
    </Alert>
  );
}

/**
 * A query's three states: a Skeleton while it first loads, a `LoadError` when it failed with nothing to show, else
 * `children(data)`. Data that loaded before a failed refetch keeps showing.
 */
export function QueryStatus<T>({ query, what, lines, skeleton, retry, className, children }: {
  query: Pick<UseQueryResult<T>, 'data' | 'error' | 'isPending' | 'refetch'>;
  /** What it loads, as the start of a sentence: "Secrets", "The run". */
  what: string;
  /** The Skeleton: this many text lines, or (without `lines`) a block sized by `skeleton` ("h-24"). */
  lines?: number;
  skeleton?: string;
  /** Offer Retry on a failure. */
  retry?: boolean;
  /** On the Skeleton and the error. */
  className?: string;
  children: (data: T) => ReactNode;
}) {
  if (query.data !== undefined) return <>{children(query.data)}</>;
  if (query.error) {
    return <LoadError what={what} error={query.error} className={className} onRetry={retry ? () => void query.refetch() : undefined} />;
  }
  if (query.isPending) return <Skeleton lines={lines} label={loadingLabel(what)} className={className ?? skeleton} />;
  return null;
}
