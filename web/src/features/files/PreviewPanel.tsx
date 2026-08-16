/**
 * Workspace file preview panel — the content of a `preview:<absPath>` tab
 * (meta from uiStore.previewTabs, mirroring the `subagent:` tab pattern).
 *
 * Loads the file once per mount via `fetchFile` and branches on the
 * FileContent kind (design D3):
 * - text + code-ish extension → gutter line numbers + shiki highlighting.
 *   The highlighter is the chat CodeBlock singleton (never a second core);
 *   line numbers are a separate CSS-aligned gutter column — shiki emits
 *   exactly one `.line` span per `split("\n")` element, so both sides count
 *   lines identically without any transformer.
 * - text > HIGHLIGHT_BYTE_LIMIT → same gutter, plain unhighlighted <pre>.
 * - text + .md → the chat Markdown renderer (GFM, fenced code via CodeBlock,
 *   raw HTML/images disabled) with a 渲染/源码 toolbar toggle.
 * - blob image/* (svg included) → object URL in <img>; revoked on unmount or
 *   content change.
 * - blob application/pdf → object URL in an <iframe> filling the panel.
 * - binary-unsupported → icon + size notice.
 * - 413/network errors → icon + the daemon's message (oversize responses
 *   already carry the real size and limit) + a retry button.
 */
import { useEffect, useMemo, useState } from "react";
import type { HighlighterCore } from "shiki/core";
import {
  Ban,
  Code2,
  Eye,
  FileText,
  FileWarning,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { formatBytes, type DaemonClient } from "../../api/client";
import type { FileContent } from "../../api/types";
import type { PreviewTabMeta } from "../../state/uiStore";
import { getHighlighter, THEME, isRegisteredLang } from "../chat/CodeBlock";
import { Markdown } from "../chat/ChatView";
import { Button } from "../../components/ui/button";
import {
  HIGHLIGHT_BYTE_LIMIT,
  isImageMime,
  isMarkdownPath,
  isPdfMime,
  langForPath,
  textBytes,
} from "./previewLogic";

export interface PreviewPanelProps {
  meta: PreviewTabMeta;
  client: DaemonClient;
}

type LoadState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "done"; content: FileContent };

export function PreviewPanel({ meta, client }: PreviewPanelProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  // Bumped by the retry button to re-run the fetch effect below.
  const [attempt, setAttempt] = useState(0);
  // 渲染/源码 toggle for .md files (code view shows the highlighted source).
  const [mdSource, setMdSource] = useState(false);

  useEffect(() => {
    let live = true;
    setState({ status: "loading" });
    client
      .fetchFile(meta.absPath)
      .then(
        (content) => {
          if (live) setState({ status: "done", content });
        },
        (e) => {
          if (live)
            setState({
              status: "error",
              message: e instanceof Error ? e.message : String(e),
            });
        },
      );
    return () => {
      live = false;
    };
  }, [client, meta.absPath, attempt]);

  // Highlighting reuses the chat CodeBlock singleton (loaded lazily; until it
  // resolves the code view falls back to plain text, like CodeBlock itself).
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

  const content = state.status === "done" ? state.content : null;

  const textInfo = useMemo(() => {
    if (!content || content.kind !== "text") return null;
    return { text: content.lines.join("\n"), bytes: textBytes(content.lines) };
  }, [content]);

  const text = textInfo?.text ?? "";
  const bytes = textInfo?.bytes ?? 0;
  const degraded = bytes > HIGHLIGHT_BYTE_LIMIT;
  // shiki line spans split exactly on "\n" — derive the gutter from the same
  // string so the two columns can never disagree.
  const lineCount = text.split("\n").length;

  const lang = langForPath(meta.relPath);
  const highlightedHtml = useMemo(() => {
    if (!highlighter || lang === null || !isRegisteredLang(lang) || degraded) return null;
    return highlighter.codeToHtml(text, { lang, theme: THEME });
  }, [highlighter, lang, degraded, text]);

  // Blob object URL lifecycle: created when the blob content arrives, revoked
  // on unmount or when the content (path/retry) changes — no leaks.
  const blob = content?.kind === "blob" ? content.blob : null;
  const [blobUrl, setBlobUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!blob) {
      setBlobUrl(null);
      return;
    }
    const url = URL.createObjectURL(blob);
    setBlobUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [blob]);

  let sizeLabel: string | null = null;
  if (content?.kind === "text" || content?.kind === "binary-unsupported") {
    sizeLabel = formatBytes(content.version.size);
  } else if (content?.kind === "blob") {
    sizeLabel = formatBytes(content.blob.size);
  }

  const isMd = isMarkdownPath(meta.relPath);

  const renderBody = () => {
    if (state.status === "loading") return <Loading />;
    if (state.status === "error") {
      return (
        <Notice
          icon={<FileWarning size={20} className="text-warning" />}
          message={state.message}
          action={
            <Button variant="outline" size="sm" onClick={() => setAttempt((n) => n + 1)}>
              <RefreshCw size={12} />
              重试
            </Button>
          }
        />
      );
    }
    const c = state.content;
    if (c.kind === "binary-unsupported") {
      return (
        <Notice
          icon={<Ban size={20} className="text-muted-foreground" />}
          message="二进制文件，暂不支持预览"
          detail={formatBytes(c.version.size)}
        />
      );
    }
    if (c.kind === "blob") {
      if (isImageMime(c.mime)) {
        return blobUrl ? (
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-4">
            <img src={blobUrl} alt={meta.relPath} className="max-h-full max-w-full object-contain" />
          </div>
        ) : (
          <Loading />
        );
      }
      if (isPdfMime(c.mime)) {
        return blobUrl ? (
          <iframe
            src={blobUrl}
            title={meta.relPath}
            className="min-h-0 w-full flex-1 border-0"
          />
        ) : (
          <Loading />
        );
      }
      // Mime outside the daemon whitelist — shouldn't happen, fail gently.
      return (
        <Notice
          icon={<Ban size={20} className="text-muted-foreground" />}
          message="二进制文件，暂不支持预览"
          detail={formatBytes(c.blob.size)}
        />
      );
    }
    if (isMd && !mdSource) {
      return (
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <div className="mx-auto max-w-3xl">
            <Markdown>{text}</Markdown>
          </div>
        </div>
      );
    }
    return <CodeView html={highlightedHtml} text={text} lineCount={lineCount} />;
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-border bg-sidebar px-3">
        <FileText size={13} className="shrink-0 text-primary" />
        <span className="min-w-0 truncate font-mono text-[12px]" title={meta.relPath}>
          {meta.relPath}
        </span>
        {sizeLabel && (
          <span className="shrink-0 text-[11px] text-muted-foreground">{sizeLabel}</span>
        )}
        {degraded && textInfo && (
          <span
            className="shrink-0 rounded-sm border border-border px-1.5 text-[10px] text-muted-foreground"
            title={`超过 ${formatBytes(HIGHLIGHT_BYTE_LIMIT)}，已跳过语法高亮`}
          >
            已跳过高亮
          </span>
        )}
        {isMd && (
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 shrink-0 gap-1 px-2 text-[11px]"
            onClick={() => setMdSource((v) => !v)}
            title={mdSource ? "切换到渲染视图" : "切换到源码视图"}
          >
            {mdSource ? <Eye size={12} /> : <Code2 size={12} />}
            {mdSource ? "渲染" : "源码"}
          </Button>
        )}
      </header>
      {renderBody()}
    </div>
  );
}

function Loading() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center text-muted-foreground">
      <Loader2 size={16} className="animate-spin" />
    </div>
  );
}

function Notice({
  icon,
  message,
  detail,
  action,
}: {
  icon: React.ReactNode;
  message: string;
  detail?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
      {icon}
      <p className="max-w-md break-words text-[13px] text-foreground">{message}</p>
      {detail && <p className="text-[11px] text-muted-foreground">{detail}</p>}
      {action}
    </div>
  );
}

/**
 * Code/text body: sticky line-number gutter + (highlighted) code column.
 * Both columns share the container's font metrics (font-mono text-[12px]
 * leading-5) — shiki's <pre> sets neither font nor line-height inline, so the
 * rows align by construction; the gutter sticks left while scrolling wide
 * lines.
 */
function CodeView({
  html,
  text,
  lineCount,
}: {
  html: string | null;
  text: string;
  lineCount: number;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-auto font-mono text-[12px] leading-5">
      <div className="flex min-h-full">
        <pre
          aria-hidden="true"
          className="sticky left-0 z-10 shrink-0 select-none border-r border-border bg-sidebar py-3 pr-2.5 pl-3 text-right text-muted-foreground/70"
        >
          {Array.from({ length: lineCount }, (_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
        </pre>
        {html !== null ? (
          // Highlighted by shiki from the file bytes — no user-controlled
          // markup survives tokenization. Keep shiki's own background so the
          // one-dark-pro palette stays self-consistent.
          <div className="min-w-0 flex-1 py-3 pl-3" dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <pre className="min-w-0 flex-1 whitespace-pre py-3 pl-3">{text}</pre>
        )}
      </div>
    </div>
  );
}
