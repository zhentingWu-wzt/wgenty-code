/**
 * Mermaid diagram renderer for fenced ```mermaid blocks in chat and preview.
 *
 * Renders flowchart (LR/TD), sequence, and other Mermaid diagram types as SVG.
 * The mermaid module is lazy-loaded on the first diagram (it is large), then
 * the shared mermaid instance is reused — mirrors the shiki highlighter
 * singleton pattern in CodeBlock.tsx.
 *
 * Streaming safety: while the assistant is still streaming, block content is
 * incomplete and mermaid's parser throws on partial syntax. Render failures
 * fall back to a plain code block (the raw text), and retry on the next
 * content change — the diagram appears the moment the fence closes and the
 * syntax becomes valid.
 *
 * Security: securityLevel "strict" — labels are sanitized, click callbacks
 * disabled; the SVG is injected via dangerouslySetInnerHTML from mermaid's
 * own sandboxed render output, matching how the markdown pipeline already
 * trusts shiki's generated HTML.
 */
import { useEffect, useRef, useState } from "react";
import type { Mermaid } from "mermaid";

/** Dark-mode detection — re-render diagrams when the theme flips. */
function useIsDark(): boolean {
  const [isDark, setIsDark] = useState(
    () => document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    const observer = new MutationObserver(() => {
      setIsDark(document.documentElement.classList.contains("dark"));
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, []);
  return isDark;
}

let mermaidPromise: Promise<Mermaid> | null = null;

/** Shared mermaid instance; lazy import keeps it out of the initial bundle. */
function getMermaid(isDark: boolean): Promise<Mermaid> {
  const key = isDark ? "dark" : "default";
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((mod) => {
      mod.default.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: key,
        // Match the app's typography scale; diagrams stay readable at chat width.
        fontFamily:
          "ui-sans-serif, system-ui, -apple-system, sans-serif",
        fontSize: 13,
      });
      return mod.default;
    });
  }
  return mermaidPromise;
}

export function MermaidBlock({ value }: { value: string }) {
  const [error, setError] = useState<string | null>(null);
  const [svg, setSvg] = useState<string>("");
  const containerRef = useRef<HTMLDivElement>(null);
  const isDark = useIsDark();

  useEffect(() => {
    let cancelled = false;
    const id = `mermaid-${Math.random().toString(36).slice(2)}`;
    getMermaid(isDark)
      .then(async (mermaid) => {
        // parse() validates before render — throws on incomplete/invalid syntax.
        await mermaid.parse(value);
        const { svg } = await mermaid.render(id, value);
        if (!cancelled) {
          setSvg(svg);
          setError(null);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setSvg("");
          setError(e instanceof Error ? e.message : String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [value, isDark]);

  if (error) {
    // Incomplete (still streaming) or invalid syntax: show the source.
    return (
      <pre className="my-2.5 overflow-x-auto rounded-md border border-border bg-background px-3.5 py-2.5 font-mono text-[0.82em] leading-normal">
        <code>{value}</code>
        <div className="mt-1 text-[10px] text-muted-foreground">
          mermaid: {error}
        </div>
      </pre>
    );
  }

  return (
    <div
      ref={containerRef}
      className="mermaid-diagram my-2.5 overflow-x-auto rounded-md border border-border bg-background p-3"
      data-error={error ?? undefined}
      // mermaid.render output — sanitized under securityLevel "strict".
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
