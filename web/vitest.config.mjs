// Vitest-only config (JS so no config bundling is needed — vite.config.ts
// would trigger a sandbox-unfriendly timestamp-file write during load).
// `vitest` prefers this file over `vite.config.ts`; dev-server proxy settings
// are irrelevant to tests. esbuild's automatic JSX replaces the react plugin
// for test transforms.
import { defineConfig } from "vitest/config";

export default defineConfig({
  esbuild: {
    jsx: "automatic",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
