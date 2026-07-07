/**
 * The Solid provider that makes the app `QueryClient` available to every
 * feature. This is the only JSX module in the package; the client, keys, and
 * error helpers stay framework-agnostic so they can be unit-tested in Node.
 */

import { QueryClientProvider, type QueryClient } from "@tanstack/solid-query";
import type { JSX } from "solid-js";
import { createAppQueryClient, type AppQueryClientOptions } from "./client";

export interface AppQueryProviderProps extends AppQueryClientOptions {
  /**
   * A pre-built client. When omitted the provider builds one from the
   * {@link AppQueryClientOptions} fields. Supplying your own is useful in tests
   * or when seeding a cache.
   */
  client?: QueryClient;
  children?: JSX.Element;
}

/**
 * Provide the app `QueryClient` to the subtree. Mount it once, above the router,
 * at the app's composition root:
 *
 * ```tsx
 * <AppQueryProvider onUnauthorized={redirectToLoginOnUnauthorized({ logout })}>
 *   <Router>…</Router>
 * </AppQueryProvider>
 * ```
 */
export function AppQueryProvider(props: AppQueryProviderProps): JSX.Element {
  const client = props.client ?? createAppQueryClient({ onUnauthorized: props.onUnauthorized });
  return <QueryClientProvider client={client}>{props.children}</QueryClientProvider>;
}

export { redirectToLoginOnUnauthorized } from "./unauthorized";
export type { RedirectOnUnauthorizedOptions } from "./unauthorized";
