import { defineConfig } from "vitest/config";

/**
 * The package's unit tests cover the framework-agnostic core — the query-key
 * factory, the api-client↔Query error bridge, and the client's 401 interceptor
 * and retry policy — so they run in a plain Node environment. The Solid
 * `provider.tsx` is intentionally excluded: it is exercised through the app.
 */
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.ts"],
  },
});
