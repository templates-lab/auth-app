/**
 * Feature contract shared by the app shell and every feature package.
 *
 * A *feature* is a workspace package that owns a slice of the admin app: the
 * routes it renders and the sidebar entries that link to them. The shell knows
 * nothing about any particular feature — it only consumes {@link FeatureModule}
 * descriptors. Adding a feature therefore means publishing a new package and
 * registering its module with the shell; no existing feature is touched.
 */

import { createContext, useContext } from "solid-js";
import type { RouteDefinition } from "@solidjs/router";

/** A single entry rendered in the admin sidebar navigation. */
export interface NavItem {
  /**
   * Absolute path the entry links to. Should match one of the feature's
   * {@link FeatureModule.routes} so the link resolves within the shell.
   */
  path: string;
  /** Text shown in the sidebar. */
  label: string;
  /** Optional leading glyph (e.g. an emoji or short symbol). */
  icon?: string;
  /**
   * Sort weight within the sidebar; lower values come first. Entries without a
   * weight fall back to {@link DEFAULT_NAV_ORDER}.
   */
  order?: number;
  /**
   * Roles allowed to see this nav entry. When absent or empty the entry is
   * visible to every authenticated user; when set, only users whose role is
   * listed will see it.
   */
  roles?: string[];
}

/**
 * The public surface a feature package exposes to the shell: the routes it
 * contributes to the router and the navigation entries that reach them.
 */
export interface FeatureModule {
  /** Stable, unique identifier for the feature (e.g. `"dashboard"`). */
  id: string;
  /** Human-readable title, used for labelling and diagnostics. */
  title: string;
  /** Routes mounted by the shell under the admin layout. */
  routes: RouteDefinition[];
  /** Sidebar entries the feature contributes. Optional for headless features. */
  nav?: NavItem[];
  /**
   * Roles allowed to access this feature's routes and nav entries. When absent
   * or empty every authenticated user has access; when set, the shell filters
   * navigation and guards routes for users whose role is not listed.
   */
  roles?: string[];
}

/** Fallback sort weight for a {@link NavItem} that omits `order`. */
export const DEFAULT_NAV_ORDER = 100;

/**
 * Identity helper for authoring a {@link FeatureModule}. It adds no behaviour —
 * it exists to give feature authors inference and a single, greppable choke
 * point the shell contract flows through.
 */
export function defineFeature(feature: FeatureModule): FeatureModule {
  return feature;
}

/**
 * Check whether a user role is allowed by the given role list. Returns `true`
 * when `allowedRoles` is `undefined` or empty (open to all authenticated
 * users), or when `userRole` is included in the array.
 */
export function isRoleAllowed(userRole: string, allowedRoles?: string[]): boolean {
  return !allowedRoles || allowedRoles.length === 0 || allowedRoles.includes(userRole);
}

// ---------------------------------------------------------------------------
// Session context — shared between the shell and every feature package.
// ---------------------------------------------------------------------------

/** Minimal session shape feature packages may depend on. */
export interface Session {
  admin_id: string;
  role: string;
  email?: string;
  display_name?: string | null;
}

/**
 * Context provided by `<RequireSession>` in the shell. Feature packages should
 * never create or provide this context — they only consume it via
 * {@link useSession} or {@link useHasRole}.
 */
export const SessionContext = createContext<() => Session>();

/**
 * The current admin's identity (id + role). Throws if used outside a
 * `<RequireSession>` subtree — a programming error, since the guard guarantees
 * a session before rendering its children.
 */
export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) {
    throw new Error("useSession must be used within <RequireSession>");
  }
  return session();
}

/**
 * Convenience hook that checks whether the authenticated user has one of the
 * given roles. Uses the session already resolved by `<RequireSession>` — no
 * extra fetch required.
 */
export function useHasRole(...roles: string[]): boolean {
  const session = useSession();
  return isRoleAllowed(session.role, roles);
}
