import { Fragment, useEffect, useRef } from "react";
import type { ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useSessionStore } from "../../state/sessionContext";
import type { DisplayMessage } from "../../state/sessionStore";
import { cn } from "../../lib/utils";
import { Button } from "../../components/ui/button";
import { RunningToolCard, ToolCallCard } from "./ToolCallCard";
import { CodeBlock } from "./CodeBlock";

/**
 * Render assistant content as GFM Markdown. Re-parses on every render — for
 * MVP-scale streamed output this is acceptable; if huge outputs become a
 * problem, throttle with requestAnimationFrame (design D3 risk note).
 *
 * Only assistant output is Markdown-rendered. User input stays plain text so a
 * user's literal typing (e.g. `## foo`) isn't misread as formatting.
 */

/**
 * react-markdown v10 routes both inline and fenced code through `code`, but
 * only fenced blocks carry a `language-xxx` className. Route fenced blocks to
 * <CodeBlock> (syntax highlighting); leave inline code to the wrapper's
 * descendant-selector styling ([&_code]:… on the Markdown container).
 */
function MarkdownCode(props: ComponentPropsWithoutRef<"code">) {
  const { className, children } = props;
  const match = /language-(\w+)/.exec(className ?? "");
  // Inline code (no language class) — render as-is; wrapper classes style it.
  if (!match) return <code className={className}>{children}</code>;

  const text = String(children).replace(/\n$/, "");
  return <CodeBlock language={match[1]} value={text} />;
}

/** GFM typography expressed as descendant selectors on the wrapper. */
const MARKDOWN_CLASSES = cn(
  "leading-relaxed",
  "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
  "[&_p]:my-2 [&_p]:leading-relaxed",
  "[&_h1]:my-3 [&_h1]:text-[1.3em] [&_h1]:font-semibold [&_h1]:leading-snug",
  "[&_h2]:my-3 [&_h2]:text-[1.15em] [&_h2]:font-semibold [&_h2]:leading-snug",
  "[&_h3]:my-3 [&_h3]:text-[1.05em] [&_h3]:font-semibold [&_h3]:leading-snug",
  "[&_h4]:my-3 [&_h4]:font-semibold [&_h4]:leading-snug",
  "[&_ul]:my-1.5 [&_ul]:list-disc [&_ul]:pl-5",
  "[&_ol]:my-1.5 [&_ol]:list-decimal [&_ol]:pl-5",
  "[&_li]:my-0.5 [&_li>p]:my-0.5",
  "[&_a]:text-primary [&_a]:no-underline [&_a:hover]:underline",
  "[&_blockquote]:my-2 [&_blockquote]:border-l-[3px] [&_blockquote]:border-border [&_blockquote]:py-0.5 [&_blockquote]:pl-3.5 [&_blockquote]:text-muted-foreground",
  "[&_code]:rounded-sm [&_code]:bg-background [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[0.85em]",
  // 移动端防溢出：内联 code（长路径/命令）与链接必须可断行，否则撑出
  // 页面级横向滚动条；表格改为块级 + 内部横滚（display:block 后内部仍按
  // 匿名 table 盒渲染，列对齐保留），宽表不再撑破布局。
  "[&_code]:[overflow-wrap:anywhere] [&_a]:break-all",
  "[&_table]:block [&_table]:overflow-x-auto [&_table]:w-max [&_table]:max-w-full",
  "[&_pre]:my-2.5 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border [&_pre]:bg-background! [&_pre]:px-3.5 [&_pre]:py-2.5",
  "[&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-[0.82em] [&_pre_code]:leading-normal",
  "[&_table]:my-2.5 [&_table]:border-collapse [&_table]:text-[0.88em]",
  "[&_th]:border [&_th]:border-border [&_th]:px-2 [&_th]:py-1 [&_th]:text-left",
  "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1",
  "[&_hr]:my-3 [&_hr]:border-border",
);

/** GFM renderer shared with the file preview panel (.md files). */
export function Markdown({ children }: { children: string }) {
  return (
    <div className={MARKDOWN_CLASSES}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{ code: MarkdownCode }}
        // Disallow raw HTML — assistant output must not inject markup. Images
        // are also disabled for now (no origin trust on tool-produced text).
        disallowedElements={["script", "style", "img", "iframe"]}
        unwrapDisallowed
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}

/**
 * Timeline-mode tool entry: a running placeholder while the tool executes,
 * a result card once tool_result arrives, and a plain-text fallback for
 * standalone tool messages from saved history that carry no parsed result.
 */
function ToolEntry({ m }: { m: DisplayMessage }) {
  if (m.streaming) return <RunningToolCard name={m.toolName ?? "tool"} args={m.toolArgs} />;
  if (m.toolExec) return <ToolCallCard exec={m.toolExec} />;
  if (m.content) {
    return (
      <div className="w-full rounded-md border border-border bg-card px-3 py-2 font-mono text-[12px] whitespace-pre-wrap text-muted-foreground">
        {m.content}
      </div>
    );
  }
  return null;
}

/** Scrolling message list with stick-to-bottom auto-scroll.
 *
 * While streaming, `messages` changes every token batch. Naively calling
 * `scrollIntoView({behavior:"smooth"})` on each change (a) queues overlapping
 * smooth animations → visible jumping, and (b) hijacks the scroll position so
 * the user cannot scroll up at all. Instead we track whether the user is
 * "pinned" near the bottom (within PIN_THRESHOLD px) via a passive scroll
 * listener, and only follow the stream while pinned — using an instant
 * scrollTop assignment (no animation to fight the user). Sending a new user
 * message re-pins the view. */
export function ChatView() {
  const messages = useSessionStore((s) => s.messages);
  const lastError = useSessionStore((s) => s.lastError);
  const bottomRef = useRef<HTMLDivElement>(null);
  // The scroll container is the <main> wrapper in App.tsx (overflow-y-auto);
  // discovered from the bottom sentinel via closest().
  const scrollerRef = useRef<HTMLElement | null>(null);
  const pinnedRef = useRef(true);
  const PIN_THRESHOLD = 80;

  // (Re)discover the scroll container when the sentinel mounts (the empty
  // state renders no sentinel, so discovery must retry once messages exist).
  useEffect(() => {
    scrollerRef.current = bottomRef.current?.closest("main") ?? null;
    const el = scrollerRef.current;
    if (!el) return;
    const onScroll = () => {
      pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < PIN_THRESHOLD;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [messages.length > 0]);

  useEffect(() => {
    // A fresh user message means a new turn is starting — re-pin so the
    // response follows even if the user had scrolled up to re-read.
    if (messages[messages.length - 1]?.role === "user") pinnedRef.current = true;
    const el = scrollerRef.current;
    if (el && pinnedRef.current) el.scrollTop = el.scrollHeight;
  }, [messages, lastError]);

  if (messages.length === 0) {
    return (
      <div className="mt-12 text-center text-muted-foreground">
        <p>Send a message to start.</p>
        <p className="mx-auto max-w-[480px] text-[12px] leading-relaxed">
          Read-only requests (e.g. “summarize README.md”) need no approval. File-writing requests
          will pop up a permission dialog.
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto flex max-w-[1100px] flex-col gap-2 px-3 pt-4 sm:px-6 sm:pt-6">
      {messages.map((m, i) => (
        <Fragment key={m.id}>
          {m.role === "user" && i > 0 && <div className="my-2 border-t border-border" />}
          <div
            className={cn(
              "flex flex-col gap-1 px-2 py-2 sm:px-4",
              m.role === "user" && "items-end",
            )}
          >
            {m.role === "tool" ? (
              <ToolEntry m={m} />
            ) : (
              <>
                <div className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
                  <span
                    className={cn(
                      "h-1.5 w-1.5 rounded-full",
                      m.role === "assistant" ? "bg-primary" : "bg-muted-foreground",
                    )}
                  />
                  {m.role}
                  {m.round && m.round > 1 ? ` · round ${m.round}` : ""}
                  {m.streaming ? " · …" : ""}
                </div>
                {m.reasoning && (
                  <details className="rounded-md border border-border bg-background text-[12px] text-muted-foreground">
                    <summary className="cursor-pointer px-2 py-1 select-none">reasoning</summary>
                    <pre className="max-h-60 overflow-y-auto px-2 pb-2 whitespace-pre-wrap">
                      {m.reasoning}
                    </pre>
                  </details>
                )}
                {m.content && (
                  <div
                    className={cn(
                      "rounded-lg px-3 py-2 text-[13px]",
                      m.role === "user"
                        ? "max-w-[85%] bg-primary/10 whitespace-pre-wrap"
                        : "bg-card",
                    )}
                  >
                    {m.role === "assistant" ? (
                      <>
                        <Markdown>{m.content}</Markdown>
                        {m.streaming && (
                          <span className="animate-[pulse-cursor_1s_infinite] text-primary">▍</span>
                        )}
                      </>
                    ) : (
                      m.content
                    )}
                  </div>
                )}
              </>
            )}
          </div>
        </Fragment>
      ))}
      {lastError && (
        <div className="flex items-center justify-between gap-2 rounded-md border border-danger bg-danger/10 px-3 py-2 font-mono text-[13px] whitespace-pre-wrap text-danger">
          <span>{lastError.message}</span>
          {lastError.retry && (
            <Button variant="destructive" size="sm" className="shrink-0" onClick={lastError.retry}>
              Retry
            </Button>
          )}
        </div>
      )}
      <div ref={bottomRef} />
    </div>
  );
}
