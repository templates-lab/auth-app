import { defineConfig } from "vitest/config";

/**
 * Test config kept separate from `vite.config.ts` on purpose: the shell's unit
 * tests cover pure composition logic (nav ordering, route flattening), so they
 * run in a plain Node environment without the Solid/JSDOM rendering stack.
 */
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
