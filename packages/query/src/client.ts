/**
 * The application `QueryClient` factory: sensible defaults plus the global 401
 * interceptor (AC: 401 en cualquier query dispara logout + redirect una sola
 * vez).
 */

import { MutationCache, QueryCache, QueryClient } from "@tanstack/solid-query";
import { isUnauthorized } from "./errors";

/** How long fetched data is considered fresh before a refetch is eligible. */
const DEFAULT_STALE_TIME_MS = 30_000;
/** Retry attempts for a failed query, excluding non-retryable errors (401). */
const MAX_QUERY_RETRIES = 2;

/** Options for {@link createAppQueryClient}. */
export interface AppQueryClientOptions {
  /**
   * Invoked when any query or mutation fails with a 401. Wire this to clear the
   * session and redirect to login. It fires **once** per client even if several
   * in-flight requests reject with 401 together, so a batch of unauthorized
   * responses triggers a single logout+redirect rather than a storm of them.
   * Build a fresh client after a successful login to re-arm the interceptor.
   */
  onUnauthorized?: () => void;
}

/**
 * Create the `QueryClient` used across the admin app.
 *
 * Defaults: a 30s `staleTime` (so revisiting a route within the window serves
 * cached data instead of refetching), no refetch on window focus (avoids
 * surprise network churn in an admin tool), up to two retries for transient
 * failures, and — crucially — **no retry on a 401**, which is a definitive
 * "you are not authenticated" answer that retrying cannot fix.
 */
export function createAppQueryClient(options: AppQueryClientOptions = {}): QueryClient {
  let handledUnauthorized = false;
  const handleError = (error: unknown): void => {
    if (!isUnauthorized(error) || handledUnauthorized) {
      return;
    }
    handledUnauthorized = true;
    options.onUnauthorized?.();
  };

  return new QueryClient({
    queryCache: new QueryCache({ onError: handleError }),
    mutationCache: new MutationCache({ onError: handleError }),
    defaultOptions: {
      queries: {
        staleTime: DEFAULT_STALE_TIME_MS,
        refetchOnWindowFocus: false,
        retry: (failureCount, error) =>
          isUnauthorized(error) ? false : failureCount < MAX_QUERY_RETRIES,
      },
      mutations: {
        retry: false,
      },
    },
  });
}
