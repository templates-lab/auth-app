/**
 * Feature contract shared by the app shell and every feature package.
 *
 * A *feature* is a workspace package that owns a slice of the admin app: the
 * routes it renders and the sidebar entries that link to them. The shell knows
 * nothing about any particular feature — it only consumes {@link FeatureModule}
 * descriptors. Adding a feature therefore means publishing a new package and
 * registering its module with the shell; no existing feature is touched.
 */

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
