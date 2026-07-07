import { Router } from "@solidjs/router";
import type { Component } from "solid-js";
import { createApiClient } from "@auth-app/api-client";
import { AppQueryProvider, redirectToLoginOnUnauthorized } from "@auth-app/query/provider";
import { AdminLayout } from "./shell/AdminLayout";
import { FeatureRoutes } from "./shell/routes";

/**
 * The single api-client instance every feature's queries fetch through. It is
 * also handed to the 401 interceptor below so an unauthorized response logs the
 * session out server-side before redirecting.
 */
const api = createApiClient();

/**
 * The application shell. It provides the shared TanStack Query client (with the
 * global 401 → logout+redirect interceptor), wraps every route in the admin
 * layout, and mounts the routes contributed by feature packages. It has no
 * knowledge of any specific feature — features are discovered through the shell
 * registry.
 */
export const App: Component = () => {
  return (
    <AppQueryProvider
      onUnauthorized={redirectToLoginOnUnauthorized({
        logout: () => api.POST("/auth/logout"),
      })}
    >
      <Router root={AdminLayout}>{FeatureRoutes()}</Router>
    </AppQueryProvider>
  );
};
