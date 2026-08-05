import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// vitest runs with globals disabled (this repo imports from "vitest"
// explicitly), so RTL's automatic cleanup never registers — do it here.
afterEach(() => cleanup());

// jsdom 未实现 matchMedia；主题解析（lib/theme.ts 的 resolveDark）和 sonner
// 的 theme="system" 都依赖它。默认按 light（matches: false）。
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }),
});
