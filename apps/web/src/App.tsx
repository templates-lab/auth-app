import { Router } from "@solidjs/router";
import type { Component } from "solid-js";
import { AdminLayout } from "./shell/AdminLayout";
import { FeatureRoutes } from "./shell/routes";

/**
 * The application shell. It wraps every route in the admin layout and mounts
 * the routes contributed by feature packages. It has no knowledge of any
 * specific feature — features are discovered through the shell registry.
 */
export const App: Component = () => {
  return <Router root={AdminLayout}>{FeatureRoutes()}</Router>;
};
