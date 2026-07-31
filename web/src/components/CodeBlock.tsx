/**
 * Syntax-highlighted code block for Markdown rendering.
 *
 * Uses react-syntax-highlighter's PrismLight so languages are registered
 * on-demand (keeps the bundle small vs the full Prism build). Synchronous
 * highlighting — important for the streaming chat UX, where shiki's async
 * grammar loading would stutter token-by-token re-rendering.
 *
 * Inline code (no language / single line, no newline) is styled by CSS only
 * (see .msg-markdown code in styles.css) and does not enter this component.
 */
import { PrismLight as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";

// Register only the languages we expect from a coding agent. Additional
// languages can be registered here as needed; each one is a small additive
// cost. (Unregistered languages fall back to plain text — no crash.)
import bash from "react-syntax-highlighter/dist/esm/languages/prism/bash";
import json from "react-syntax-highlighter/dist/esm/languages/prism/json";
import rust from "react-syntax-highlighter/dist/esm/languages/prism/rust";
import typescript from "react-syntax-highlighter/dist/esm/languages/prism/typescript";
import javascript from "react-syntax-highlighter/dist/esm/languages/prism/javascript";
import python from "react-syntax-highlighter/dist/esm/languages/prism/python";
import toml from "react-syntax-highlighter/dist/esm/languages/prism/toml";
import yaml from "react-syntax-highlighter/dist/esm/languages/prism/yaml";
import markdown from "react-syntax-highlighter/dist/esm/languages/prism/markdown";
import css from "react-syntax-highlighter/dist/esm/languages/prism/css";

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

const REGISTERED: Record<string, unknown> = {
  bash,
  json,
  rust,
  typescript,
  javascript,
  python,
  toml,
  yaml,
  markdown,
  css,
};

for (const [name, mod] of Object.entries(REGISTERED)) {
  SyntaxHighlighter.registerLanguage(name, mod as never);
}

export interface CodeBlockProps {
  language: string | null;
  value: string;
}

export function CodeBlock({ language, value }: CodeBlockProps) {
  const resolved = language ? (ALIASES[language] ?? language) : "text";
  const isRegistered = language ? resolved in REGISTERED : false;

  // If the language isn't registered, render plain preformatted text rather
  // than letting PrismLight guess — predictable output beats surprising output.
  if (!isRegistered) {
    return (
      <pre className="msg-markdown-pre-unknown">
        <code>{value}</code>
      </pre>
    );
  }

  return (
    <SyntaxHighlighter
      language={resolved}
      style={oneDark}
      // Match the .msg-markdown pre styling; useInlineStyles carries the theme.
      customStyle={{
        margin: "0.6em 0",
        padding: "0.7em 0.9em",
        background: "var(--bg)",
        border: "1px solid var(--border)",
        borderRadius: "6px",
        fontSize: "0.82em",
      }}
      codeTagProps={{ style: { fontFamily: undefined } }}
    >
      {value}
    </SyntaxHighlighter>
  );
}
