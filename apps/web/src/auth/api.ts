/**
 * The auth screens' data layer: the login mutation, the OAuth provider list,
 * and the current-session query. Kept in the shell (not a sidebar feature) —
 * signing in is a precondition for the app, not a slice of it.
 */

import { createApiClient, type components } from "@auth-app/api-client";
import { createFeatureKeys, unwrap } from "@auth-app/query";

/** The authenticated admin's identity (id + role). */
export type Me = components["schemas"]["MeOut"];

/** The auth query-key namespace, shared by the login screen and the guards. */
export const authKeys = createFeatureKeys("auth");

/** The key the current-session query lives under. */
export const meKey = () => authKeys.detail("me");

// One stateless client for the shell's auth calls.
const api = createApiClient();

/**
 * Sign in with an email and password. On success the backend sets the session
 * and CSRF cookies; the resolved value is the admin's id. A failure throws an
 * `ApiError` whose status the caller maps to a *generic* message — the backend
 * never distinguishes "no such account" from "wrong password", and neither does
 * the UI.
 */
export function login(email: string, password: string): Promise<components["schemas"]["LoginOk"]> {
  return unwrap(api.POST("/auth/login", { body: { email, password } }));
}

/** Log out server-side (best-effort). Sends the CSRF header the mutation needs. */
export function logout(): Promise<unknown> {
  return api.POST("/auth/logout", { headers: csrfHeader() });
}

/** The authenticated admin, or a thrown `ApiError(401)` when no session holds. */
export function getMe(): Promise<Me> {
  return unwrap(api.GET("/auth/me"));
}

/**
 * The enabled OAuth provider ids. Resolves to `[]` when OAuth is disabled (the
 * route is absent, a 404) or otherwise unreachable — the login screen then
 * simply shows no provider buttons.
 */
export async function listProviders(): Promise<string[]> {
  try {
    const out = await unwrap(api.GET("/auth/oauth/providers"));
    return out.providers;
  } catch {
    return [];
  }
}

/** The browser URL that begins an OAuth flow for `provider` (a full redirect). */
export function oauthStartUrl(provider: string): string {
  return `/api/auth/oauth/${encodeURIComponent(provider)}/start`;
}

/** The `X-CSRF-Token` header mirrored from the readable `csrf` cookie. */
function csrfHeader(): Record<string, string> {
  const token = document.cookie
    .split("; ")
    .find((row) => row.startsWith("csrf="))
    ?.slice("csrf=".length);
  return token ? { "x-csrf-token": token } : {};
}
