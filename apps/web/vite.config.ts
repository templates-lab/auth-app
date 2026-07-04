import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  server: {
    port: 5173,
  },
  resolve: {
    // A single Solid runtime instance across the app and every feature package.
    dedupe: ["solid-js", "@solidjs/router"],
  },
  optimizeDeps: {
    // Feature packages ship Solid source (`.tsx`); keep them out of the esbuild
    // pre-bundle so vite-plugin-solid compiles their JSX with the app's.
    exclude: ["@auth-app/feature-dashboard", "@auth-app/feature-users", "@auth-app/feature-kit"],
  },
});
