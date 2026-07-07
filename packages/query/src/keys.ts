/**
 * Typed, centralised query-key factory (AC: query keys tipadas y centralizadas
 * por feature).
 *
 * Every feature owns exactly one key namespace built by {@link createFeatureKeys}
 * and exports it from its package. Because keys are derived from a single source
 * — never spelled out as ad-hoc arrays in components — invalidation stays
 * precise: a mutation invalidates `keys.all` to refetch everything the feature
 * owns, `keys.lists()` for just its collections, or `keys.detail(id)` for one
 * record. The factory follows the conventional hierarchical shape so a broader
 * key is always a prefix of the narrower ones nested under it, which is exactly
 * what `queryClient.invalidateQueries({ queryKey })` matches against.
 */

/** A read-only query key: the array TanStack Query hashes to identify a query. */
export type QueryKey = readonly unknown[];

/** The set of keys a feature exposes. See {@link createFeatureKeys}. */
export interface FeatureKeys<F extends string> {
  /** Root key — invalidate to refetch every query the feature owns. */
  readonly all: readonly [F];
  /** Key prefix shared by all of the feature's list/collection queries. */
  lists(): readonly [F, "list"];
  /** Key for a specific list, optionally narrowed by filter parameters. */
  list(
    params?: Record<string, unknown>,
  ): readonly [F, "list"] | readonly [F, "list", Record<string, unknown>];
  /** Key prefix shared by all of the feature's single-entity queries. */
  details(): readonly [F, "detail"];
  /** Key for a single entity by id. */
  detail(id: string | number): readonly [F, "detail", string | number];
}

/**
 * Build the query-key namespace for one feature. `feature` should be the
 * feature's stable id (e.g. `"users"`), matching {@link FeatureModule.id} so the
 * keys are greppable back to their owner.
 */
export function createFeatureKeys<const F extends string>(feature: F): FeatureKeys<F> {
  const all = [feature] as const;
  const lists = () => [feature, "list"] as const;
  const details = () => [feature, "detail"] as const;
  return {
    all,
    lists,
    list: (params?: Record<string, unknown>) =>
      params === undefined ? lists() : ([feature, "list", params] as const),
    details,
    detail: (id: string | number) => [feature, "detail", id] as const,
  };
}
