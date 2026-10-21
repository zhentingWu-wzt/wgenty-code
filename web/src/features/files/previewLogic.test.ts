import { describe, expect, it } from "vitest";
import {
  EXT_TO_LANG,
  HIGHLIGHT_BYTE_LIMIT,
  extensionOf,
  isImageMime,
  isMarkdownPath,
  isPdfMime,
  langForPath,
  textBytes,
} from "./previewLogic";
import { isRegisteredLang } from "../chat/CodeBlock";

describe("extensionOf", () => {
  it("extracts a lowercase extension", () => {
    expect(extensionOf("/w/proj/src/main.RS")).toBe("rs");
    expect(extensionOf("app.TSX")).toBe("tsx");
  });

  it("treats dotfiles and extensionless names as empty", () => {
    expect(extensionOf("/w/proj/.gitignore")).toBe("");
    expect(extensionOf("/w/proj/Makefile")).toBe("");
  });

  it("ignores dots in directory names", () => {
    expect(extensionOf("/w/proj/v1.2/mod.ts")).toBe("ts");
  });
});

describe("langForPath", () => {
  it("maps common code extensions to shiki languages", () => {
    expect(langForPath("src/main.rs")).toBe("rust");
    expect(langForPath("app.tsx")).toBe("typescript");
    expect(langForPath("scripts/run.sh")).toBe("bash");
    expect(langForPath("Cargo.toml")).toBe("toml");
  });

  it("returns null for unknown/unregistered languages (plaintext fallback)", () => {
    expect(langForPath("main.go")).toBeNull();
    expect(langForPath("Dockerfile")).toBeNull();
    expect(langForPath(".gitignore")).toBeNull();
  });

  it("keeps every mapped language registered in the CodeBlock highlighter", () => {
    // codeToHtml throws on unregistered langs — the mapping must never drift
    // away from CodeBlock's registration set.
    for (const lang of Object.values(EXT_TO_LANG)) {
      expect(isRegisteredLang(lang), lang).toBe(true);
    }
  });
});

describe("mime predicates", () => {
  it("recognizes image mimes including svg, case-insensitively", () => {
    expect(isImageMime("image/png")).toBe(true);
    expect(isImageMime("image/svg+xml")).toBe(true);
    expect(isImageMime("IMAGE/JPEG")).toBe(true);
    expect(isImageMime("application/pdf")).toBe(false);
    expect(isImageMime("")).toBe(false);
  });

  it("recognizes pdf only", () => {
    expect(isPdfMime("application/pdf")).toBe(true);
    expect(isPdfMime("application/PDF")).toBe(true);
    expect(isPdfMime("image/png")).toBe(false);
  });
});

describe("markdown detection and byte accounting", () => {
  it("detects markdown by extension", () => {
    expect(isMarkdownPath("docs/README.md")).toBe(true);
    expect(isMarkdownPath("notes.markdown")).toBe(true);
    expect(isMarkdownPath("src/lib.rs")).toBe(false);
  });

  it("counts UTF-8 bytes of newline-joined lines, not UTF-16 units", () => {
    expect(textBytes([])).toBe(0);
    expect(textBytes(["a", "b"])).toBe(3); // "a\nb"
    expect(textBytes(["héllo"])).toBe(6); // "é" encodes to 2 bytes, the rest to 1
  });

  it("degrades highlighting above 256KB", () => {
    expect(HIGHLIGHT_BYTE_LIMIT).toBe(256 * 1024);
    expect(textBytes(["x".repeat(256 * 1024)])).toBe(HIGHLIGHT_BYTE_LIMIT);
    expect(textBytes(["x".repeat(256 * 1024 + 1)])).toBeGreaterThan(HIGHLIGHT_BYTE_LIMIT);
  });
});
