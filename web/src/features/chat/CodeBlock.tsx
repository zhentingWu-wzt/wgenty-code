/**
 * Syntax-highlighted code block for Markdown rendering.
 *
 * Uses shiki (the same TextMate engine as VS Code) via `shiki/core` with
 * per-language imports and the JavaScript regex engine — importing the full
 * `shiki` bundle instead would emit ~300 chunks (~10 MB) of unused grammars
 * and the Oniguruma WASM. The highlighter is created once at module load — an
 * async one-time cost — after which `codeToHtml` is fully synchronous, so the
 * streaming chat UX never stutters on token-by-token re-renders. Until the
 * highlighter finishes initializing, blocks fall back to plain <pre> text
 * (identical to the unregistered-language path).
 *
 * Inline code (no language / single line, no newline) is styled by the
 * Markdown wrapper's descendant-selector classes (see MARKDOWN_CLASSES in
 * ChatView.tsx) and does not enter this component.
 */
import { useEffect, useState } from "react";
import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

let highlighterPromise: Promise<HighlighterCore> | null = null;

function getHighlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighterCore({
      themes: [import("shiki/dist/themes/one-dark-pro.mjs")],
      // Register only the languages we expect from a coding agent. Additional
      // languages can be added here as needed; each one is a small additive
      // cost. (Unregistered languages fall back to plain text — no crash.)
      langs: [
        import("shiki/dist/langs/bash.mjs"),
        import("shiki/dist/langs/json.mjs"),
        import("shiki/dist/langs/rust.mjs"),
        import("shiki/dist/langs/typescript.mjs"),
        import("shiki/dist/langs/javascript.mjs"),
        import("shiki/dist/langs/python.mjs"),
        import("shiki/dist/langs/toml.mjs"),
        import("shiki/dist/langs/yaml.mjs"),
        import("shiki/dist/langs/markdown.mjs"),
        import("shiki/dist/langs/css.mjs"),
      ],
      engine: createJavaScriptRegexEngine(),
    });
  }
  return highlighterPromise;
}

const THEME = "one-dark-pro";

/** Languages registered in `getHighlighter` — keep the two lists in sync. */
const LANGS = [
  "bash",
  "json",
  "rust",
  "typescript",
  "javascript",
  "python",
  "toml",
  "yaml",
  "markdown",
  "css",
] as const;

// Alias registry: common alternative fence labels -> registered language.
const ALIASES: Record<string, string> = {
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  py: "python",
  rs: "rust",
  yml: "yaml",
  md: "markdown",
};

// Kick off initialization immediately — by the time the first assistant
// message streams in, highlighting is almost always ready.
getHighlighter();

export interface CodeBlockProps {
  language: string | null;
  value: string;
}

export function CodeBlock({ language, value }: CodeBlockProps) {
  const [highlighter, setHighlighter] = useState<HighlighterCore | null>(null);

  useEffect(() => {
    let live = true;
    getHighlighter().then((h) => {
      if (live) setHighlighter(h);
    });
    return () => {
      live = false;
    };
  }, []);

  const resolved = language ? (ALIASES[language] ?? language) : "text";
  const isRegistered = language ? (LANGS as readonly string[]).includes(resolved) : false;

  // If the language isn't registered (or the highlighter is still loading),
  // render plain preformatted text rather than guessing — predictable output
  // beats surprising output.
  // Both branches rely on the Markdown wrapper's [&_pre]/[&_pre_code]
  // descendant classes (MARKDOWN_CLASSES in ChatView.tsx) for container
  // styling — including the important bg-background that overrides shiki's
  // inline background-color.
  if (!isRegistered || !highlighter) {
    return (
      <pre>
        <code>{value}</code>
      </pre>
    );
  }

  const html = highlighter.codeToHtml(value, { lang: resolved, theme: THEME });

  // shiki emits a complete <pre class="shiki"> element. The HTML is generated
  // by shiki from the code string itself — no user-controlled markup survives
  // tokenization.
  return <div dangerouslySetInnerHTML={{ __html: html }} />;
}
