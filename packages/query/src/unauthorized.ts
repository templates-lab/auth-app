/**
 * The default 401 handler wired into {@link createAppQueryClient} at the app's
 * composition root. Kept free of JSX so it is importable (and unit-testable) in
 * a plain Node environment, separate from the Solid provider component.
 */

/** Options for {@link redirectToLoginOnUnauthorized}. */
export interface RedirectOnUnauthorizedOptions {
  /** Path the browser is sent to after logout. Defaults to `/login`. */
  loginPath?: string;
  /**
   * Best-effort server-side logout (e.g. `() => api.POST("/auth/logout")`). Its
   * result — success or failure — never blocks the redirect; the session cookie
   * is cleared server-side when it succeeds, and the redirect discards client
   * state regardless.
   */
  logout?: () => Promise<unknown>;
}

/**
 * Build an `onUnauthorized` handler that clears the session and hard-redirects
 * to the login page, preserving the current location as a `?next=` parameter so
 * the login screen can return there after a successful sign-in. A full-page
 * navigation (not a router push) is deliberate: it discards every cached query
 * and all in-memory state, so no stale authenticated data survives the logout —
 * which is exactly what makes a mid-session expiry reflect as a logout across
 * the whole UI.
 */
export function redirectToLoginOnUnauthorized(
  options: RedirectOnUnauthorizedOptions = {},
): () => void {
  const loginPath = options.loginPath ?? "/login";
  return () => {
    const target = loginTarget(loginPath);
    const redirect = () => {
      window.location.assign(target);
    };
    if (options.logout) {
      void Promise.resolve(options.logout())
        .catch(() => undefined)
        .finally(redirect);
    } else {
      redirect();
    }
  };
}

/**
 * The login URL to send the browser to, carrying the current path as `next` so
 * sign-in can return to it. No `next` is added when there is no meaningful
 * origin or we are already on (or heading to) the login page — that would just
 * stack a redirect onto itself.
 */
function loginTarget(loginPath: string): string {
  const loc = typeof window === "undefined" ? undefined : window.location;
  const current = `${loc?.pathname ?? ""}${loc?.search ?? ""}`;
  if (!current || loc?.pathname === loginPath || current === loginPath) {
    return loginPath;
  }
  return `${loginPath}?next=${encodeURIComponent(current)}`;
}
