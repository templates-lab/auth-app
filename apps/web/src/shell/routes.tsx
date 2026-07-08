import { Route, type RouteDefinition } from "@solidjs/router";
import type { JSX, ParentProps } from "solid-js";
import { features } from "./registry";
import { NotFound } from "./NotFound";
import { RequireRole } from "./RequireRole";

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
 *
 * Features that declare `roles` are wrapped in a {@link RequireRole} guard so
 * that direct URL access by an unauthorised role shows a 403 view instead of
 * the feature's content. Features without `roles` are rendered without a guard.
 */
export function FeatureRoutes(): JSX.Element {
  return (
    <>
      {features.map((feature) => {
        const routes = toRouteElements(feature.routes);
        if (feature.roles && feature.roles.length > 0) {
          const roles = feature.roles;
          return (
            <Route
              path=""
              component={(props: ParentProps) => (
                <RequireRole roles={roles}>{props.children}</RequireRole>
              )}
            >
              {routes}
            </Route>
          );
        }
        return routes;
      })}
      <Route path="*" component={NotFound} />
    </>
  );
}
