/**
 * Pure render-kind decisions for the workspace file preview panel
 * (features/files/PreviewPanel.tsx). Kept free of React/daemon imports so the
 * extension→language mapping and mime predicates stay unit-testable.
 *
 * Language ids must stay within the set registered by chat/CodeBlock's
 * highlighter singleton — `codeToHtml` throws on unregistered langs, and the
 * panel reuses that singleton rather than creating its own. Unknown
 * extensions map to null = plaintext.
 */

/** Above this many bytes the code view skips shiki highlighting and renders
 *  plain text (design 1.4: tokenizing multi-hundred-KB files janks the main
 *  thread; 256KB is the agreed degradation threshold). */
export const HIGHLIGHT_BYTE_LIMIT = 256 * 1024;

/** Lowercase extension of a path ("" when none). Leading-dot files such as
 *  `.gitignore` are treated as extensionless — their "extension" is the
 *  whole filename. */
export function extensionOf(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "";
  return name.slice(dot + 1).toLowerCase();
}

/** Extension → shiki language id. Values limited to CodeBlock's registered
 *  languages (see its LANGS list) plus their file extensions/aliases.
 *  Exported for the sync test against CodeBlock's registration set. */
export const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  cts: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  py: "python",
  pyi: "python",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  json: "json",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  markdown: "markdown",
  css: "css",
};

/** shiki language for a path, or null when the file renders as plaintext
 *  (unknown/unregistered extension — predictable output beats guessing). */
export function langForPath(path: string): string | null {
  return EXT_TO_LANG[extensionOf(path)] ?? null;
}

/** Markdown files get the rendered view + 渲染/源码 toolbar toggle. */
export function isMarkdownPath(path: string): boolean {
  const ext = extensionOf(path);
  return ext === "md" || ext === "markdown";
}

/** Daemon image whitelist (png/jpg/gif/webp/svg) all arrive as `image/*` —
 *  including svg (`image/svg+xml`), which is safe under `<img>` because the
 *  image element never executes embedded scripts. */
export function isImageMime(mime: string): boolean {
  return mime.toLowerCase().startsWith("image/");
}

export function isPdfMime(mime: string): boolean {
  return mime.toLowerCase() === "application/pdf";
}

/** UTF-8 byte size of the joined lines. The >256KB highlight decision needs
 *  bytes, not UTF-16 code units (`"é".length === 1` but encodes to 2). */
export function textBytes(lines: string[]): number {
  return new TextEncoder().encode(lines.join("\n")).length;
}
