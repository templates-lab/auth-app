import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  server: {
    port: 5173,
  },
  resolve: {
    // A single Solid runtime instance across the app and every feature package.
    // `@tanstack/solid-query` is deduped too: its context is only shared when
    // every package resolves to the same module instance.
    dedupe: ["solid-js", "@solidjs/router", "@tanstack/solid-query"],
  },
  optimizeDeps: {
    // Feature packages ship Solid source (`.tsx`); keep them out of the esbuild
    // pre-bundle so vite-plugin-solid compiles their JSX with the app's. The
    // query package's `./provider` entry ships `.tsx` for the same reason.
    exclude: [
      "@auth-app/feature-dashboard",
      "@auth-app/feature-users",
      "@auth-app/feature-kit",
      "@auth-app/query",
    ],
  },
});
