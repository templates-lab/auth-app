/**
 * Shared, framework-agnostic building blocks for the auth-app frontend.
 * Feature packages under `packages/` and apps under `apps/` consume these.
 */

/** A minimal authenticated user shape shared across the app. */
export interface AuthUser {
  id: string;
  email: string;
  displayName?: string;
}

/** Whether the given string is a syntactically plausible email address. */
export function isValidEmail(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

/** A friendly label for a user, preferring the display name over the email. */
export function userLabel(user: AuthUser): string {
  return user.displayName ?? user.email;
}
