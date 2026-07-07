import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

/**
 * Test config kept separate from `vite.config.ts` on purpose: the shell's unit
 * tests cover pure composition logic (nav ordering, route flattening), so they
 * run in a plain Node environment without the Solid/JSDOM rendering stack.
 *
 * Workspace libraries are aliased to their TypeScript source: their
 * `package.json` `exports` point at `dist`, which the CI frontend job builds
 * only *after* the test step — so resolving to source keeps `pnpm test`
 * working from a clean checkout with no prior build.
 */
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
  resolve: {
    alias: {
      "@auth-app/feature-kit": fileURLToPath(
        new URL("../../packages/feature-kit/src/index.ts", import.meta.url),
      ),
      "@auth-app/shared": fileURLToPath(
        new URL("../../packages/shared/src/index.ts", import.meta.url),
      ),
    },
  },
});
