/**
 * The typed API client (bead authapp-a05b92).
 *
 * `schema.ts` is generated from the backend's OpenAPI spec (`pnpm gen:api` at
 * the repo root) — the contract is the only boundary between front and back,
 * with no hand-maintained duplicate types. This module wraps `openapi-fetch`
 * with that schema so every path, request body, query param, and response is
 * checked against the spec at compile time.
 */

import createClient, { type Client } from "openapi-fetch";
import type { paths } from "./schema";

/** Options for {@link createApiClient}. */
export interface ApiClientOptions {
  /**
   * Base URL the API is served from. Defaults to `/api` — the path Traefik
   * routes to the backend (it strips the prefix before the request reaches
   * axum), so the same-origin admin app needs no configuration.
   */
  baseUrl?: string;
}

/**
 * Build a typed client bound to the backend's OpenAPI schema.
 *
 * `credentials: "include"` sends the `HttpOnly` session cookie on every call,
 * which is how the cookie-based session (see the backend's session docs) is
 * carried — the client never handles a token itself.
 */
export function createApiClient(options: ApiClientOptions = {}): Client<paths> {
  return createClient<paths>({
    baseUrl: options.baseUrl ?? "/api",
    credentials: "include",
  });
}

export type { paths } from "./schema";
