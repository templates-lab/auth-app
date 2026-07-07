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
 * to the login page. A full-page navigation (not a router push) is deliberate:
 * it discards every cached query and all in-memory state, so no stale
 * authenticated data can survive the logout.
 */
export function redirectToLoginOnUnauthorized(
  options: RedirectOnUnauthorizedOptions = {},
): () => void {
  const loginPath = options.loginPath ?? "/login";
  return () => {
    const redirect = () => {
      window.location.assign(loginPath);
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
