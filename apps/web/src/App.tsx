import { Route, Router } from "@solidjs/router";
import type { Component } from "solid-js";
import { createApiClient } from "@auth-app/api-client";
import { AppQueryProvider, redirectToLoginOnUnauthorized } from "@auth-app/query/provider";
import { AdminLayout } from "./shell/AdminLayout";
import { FeatureRoutes } from "./shell/routes";
import { Login } from "./auth/Login";

/**
 * The single api-client instance every feature's queries fetch through. It is
 * also handed to the 401 interceptor below so an unauthorized response logs the
 * session out server-side before redirecting.
 */
const api = createApiClient();

/**
 * The application shell. It provides the shared TanStack Query client (with the
 * global 401 → logout+redirect interceptor) and defines the two route groups:
 * the chrome-less `/login` screen, and everything else nested under the admin
 * layout. Feature routes are discovered through the shell registry — the shell
 * has no knowledge of any specific feature.
 */
export const App: Component = () => {
  return (
    <AppQueryProvider
      onUnauthorized={redirectToLoginOnUnauthorized({
        logout: () => api.POST("/auth/logout"),
      })}
    >
      <Router>
        <Route path="/login" component={Login} />
        <Route path="/" component={AdminLayout}>
          {FeatureRoutes()}
        </Route>
      </Router>
    </AppQueryProvider>
  );
};
