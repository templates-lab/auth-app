/**
 * `@auth-app/query` — the app's TanStack Query configuration, shared by every
 * feature package.
 *
 * It owns the cross-cutting concerns of data fetching so no feature has to
 * (re)invent them: a `QueryClient` with sensible defaults and a global 401
 * interceptor ({@link createAppQueryClient}), a typed per-feature query-key
 * factory ({@link createFeatureKeys}), and the api-client↔Query bridge
 * ({@link unwrap} / {@link ApiError}). The Solid `<AppQueryProvider>` lives at
 * the `./provider` entry point to keep this barrel free of JSX.
 */

export { ApiError, isUnauthorized, unwrap, type FetchResult } from "./errors";
export { createFeatureKeys, type FeatureKeys, type QueryKey } from "./keys";
export { createAppQueryClient, type AppQueryClientOptions } from "./client";
export { redirectToLoginOnUnauthorized, type RedirectOnUnauthorizedOptions } from "./unauthorized";
