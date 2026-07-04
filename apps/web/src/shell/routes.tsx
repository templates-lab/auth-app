import { Route, type RouteDefinition } from "@solidjs/router";
import type { JSX } from "solid-js";
import { features } from "./registry";
import { NotFound } from "./NotFound";

/** Recursively turn feature route definitions into `<Route>` elements. */
function toRouteElements(defs: RouteDefinition[]): JSX.Element {
  return defs.map((def) => {
    const nested = def.children
      ? toRouteElements(Array.isArray(def.children) ? def.children : [def.children])
      : undefined;
    return (
      <Route path={def.path} component={def.component} matchFilters={def.matchFilters}>
        {nested}
      </Route>
    );
  });
}

/**
 * Every registered feature's routes, plus a catch-all fallback, ready to be
 * mounted as children of the `<Router>`. The shell never names a feature here —
 * it renders whatever {@link features} contributes.
 */
export function FeatureRoutes(): JSX.Element {
  return (
    <>
      {toRouteElements(features.flatMap((feature) => feature.routes))}
      <Route path="*" component={NotFound} />
    </>
  );
}
