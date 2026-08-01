import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// vitest runs with globals disabled (this repo imports from "vitest"
// explicitly), so RTL's automatic cleanup never registers — do it here.
afterEach(() => cleanup());
