import { useCallback, useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AlertCircle, Loader2, Network, RefreshCw } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type {
  ChatMessage,
  LocalAgentViewResponse,
} from "../../api/types";
import {
  buildAncestorChain,
  useSubagentTraceStore,
} from "../../state/subagentTraceStore";
import {
  directoryAncestorChain,
  useSubagentDirectoryStore,
} from "../../state/subagentDirectoryStore";
import { useUiStore } from "../../state/uiStore";
import { cn } from "../../lib/utils";
import { Button } from "../../components/ui/button";
import { RunningToolCard, ToolCallCard } from "../chat/ToolCallCard";

/**
 * Subagent detail panel - the content of a `subagent:<nodeId>` tab.
 *
 * Loads the subagent's local view (self + direct children) by walking the
 * capability-scoped agent API from the root session down to the target:
 *   getAgentSelf -> navigateAgentView (per ancestor level) -> target view.
 * The ancestor chain is reconstructed from the per-session directory tree
 * (durable across turns, reloads, and trace-store clears); the live trace
 * tree is only a fallback for nodes not yet present in the last directory
 * poll. Only when both sources lack the node does the panel report it as not
 * found in the session's agent tree.
 *
 * Self messages are rendered as a read-only transcript. Direct children are
 * listed and open their own detail tab on click. transcript/cancel are
 * best-effort (the daemon only grants them for root-direct children).
 */
interface Props {
  client: DaemonClient;
  nodeId: string;
  rootSessionId: string;
  label: string;
}

const REFRESH_MS = 3000;

function statusColor(status: string): string {
  const s = status.toLowerCase();
  if (s === "running" || s === "thinking") return "text-green-500";
  if (s === "completed" || s === "done") return "text-muted-foreground";
  if (s === "failed" || s === "error" || s === "cancelled") return "text-danger";
  if (s.includes("await")) return "text-warning";
  return "text-muted-foreground";
}

function isTerminal(status: string): boolean {
  const s = status.toLowerCase();
  return (
    s === "completed" ||
    s === "done" ||
    s === "failed" ||
    s === "error" ||
    s === "cancelled"
  );
}

export function SubagentDetailPanel({ client, nodeId, rootSessionId, label }: Props) {
  const liveNode = useSubagentTraceStore((s) => s.nodes.get(nodeId));
  const openSubagentTab = useUiStore((s) => s.openSubagentTab);

  const [view, setView] = useState<LocalAgentViewResponse | null>(null);
  const [targetCap, setTargetCap] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<unknown>(null);
  const [transcriptLoading, setTranscriptLoading] = useState(false);
  const [transcriptError, setTranscriptError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    // Prefer the durable per-session directory tree; fall back to the volatile
    // trace store when the directory has not loaded yet or is missing a
    // just-spawned node (poll lag).
    const dirTree =
      useSubagentDirectoryStore.getState().bySession[rootSessionId]?.tree ??
      null;
    let chain = directoryAncestorChain(dirTree, nodeId);
    if (chain.length === 0) {
      chain = buildAncestorChain(
        useSubagentTraceStore.getState().nodes,
        nodeId,
      );
    }
    if (chain.length === 0) {
      setError("未在当前会话的代理树中找到该 subagent（会话可能已重启或记录已被清理）。");
      setView(null);
      setLoading(false);
      return;
    }
    try {
      let v = await client.getAgentSelf(rootSessionId);
      let parentChildren = v.children;
      let cap: string | null = null;
      for (let i = 1; i < chain.length; i++) {
        const child = parentChildren.find((c) => c.agent_id === chain[i]);
        if (!child) throw new Error(`在父级子代理中未找到 ${chain[i]}`);
        cap = child.navigation_capability;
        v = await client.navigateAgentView(rootSessionId, child.navigation_capability);
        parentChildren = v.children;
      }
      setView(v);
      setTargetCap(chain.length > 1 ? cap : null);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [client, nodeId, rootSessionId]);

  // Auto-scroll to the latest message. Depends on a content-derived key, not
  // the messages array itself: polling replaces the array reference every
  // REFRESH_MS, and a reference dep would yank the user back to the bottom
  // even when nothing new arrived.
  const messages = view?.self_view.messages ?? [];
  const lastMessage = messages[messages.length - 1];
  const scrollKey = `${messages.length}:${lastMessage?.content?.length ?? 0}:${
    lastMessage?.tool_calls?.length ?? 0
  }`;
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [scrollKey]);

  // Initial load on mount. Switching tabs remounts via `key={nodeId}` (App),
  // so state resets naturally. Fetching remote data on mount is a legitimate
  // effect use (React docs); setState happens inside `load`, so we disable the
  // local rule (same pattern as DirectoryPickerModal).
  const loadInitial = useCallback(() => {
    void load();
  }, [load]);
  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(loadInitial, [loadInitial]);

  // Poll while the subagent is still active; stop once terminal.
  const status = liveNode?.status ?? view?.self_view.status ?? "";
  useEffect(() => {
    if (!status || isTerminal(status)) return;
    const t = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(t);
  }, [load, status]);

  const loadTranscript = async () => {
    if (!targetCap) return;
    setTranscriptLoading(true);
    setTranscriptError(null);
    try {
      const r = await client.getChildTranscript(rootSessionId, targetCap);
      setTranscript(r.transcript);
    } catch (e) {
      setTranscriptError(e instanceof Error ? e.message : String(e));
    } finally {
      setTranscriptLoading(false);
    }
  };

  const cancel = async () => {
    if (!targetCap) return;
    setCancelling(true);
    try {
      await client.cancelChild(rootSessionId, targetCap);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCancelling(false);
    }
  };

  if (loading && !view) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
        <Loader2 size={16} className="animate-spin" /> 加载中…
      </div>
    );
  }
  if (error && !view) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-muted-foreground">
        <AlertCircle size={20} />
        <span className="max-w-[420px] text-[13px]">{error}</span>
        <Button variant="outline" size="sm" onClick={() => { setLoading(true); void load(); }}>
          <RefreshCw size={13} /> 重试
        </Button>
      </div>
    );
  }
  if (!view) return null;

  const self = view.self_view;
  const curStatus = liveNode?.status ?? self.status;
  const elapsed = liveNode?.elapsedMs ?? self.elapsed_ms;
  const tokens = liveNode?.cumulativeTokens ?? self.cumulative_tokens;
  const terminal = isTerminal(curStatus);

  return (
    <div className="mx-auto flex max-w-[1100px] flex-col gap-3 px-3 py-4 sm:px-6">
      {/* Header */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <Network size={16} className="shrink-0 text-primary" />
          <h2 className="truncate text-[15px] font-semibold">{self.label || label}</h2>
          <span className={cn("shrink-0 text-[12px]", statusColor(curStatus))}>
            {curStatus}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {targetCap && !terminal && (
            <Button
              variant="outline"
              size="sm"
              disabled={cancelling}
              onClick={() => void cancel()}
            >
              {cancelling ? "取消中…" : "取消"}
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={() => void load()}>
            <RefreshCw size={13} /> 刷新
          </Button>
        </div>
      </div>

      {/* Meta */}
      <div className="flex flex-wrap gap-3 text-[12px] text-muted-foreground">
        <span>⏱ {(elapsed / 1000).toFixed(1)}s</span>
        {tokens > 0 && <span>{(tokens / 1000).toFixed(1)}k tokens</span>}
        {self.round != null && (
          <span>
            round {self.round}
            {self.max_rounds ? `/${self.max_rounds}` : ""}
          </span>
        )}
        {liveNode?.currentTool && (
          <span className="text-primary">🔧 {liveNode.currentTool}</span>
        )}
      </div>

      {/* Messages */}
      <SubagentMessages messages={self.messages} textSnapshot={self.text_snapshot} />

      {/* Direct children */}
      {view.children.length > 0 && (
        <div className="flex flex-col gap-1">
          <div className="text-[12px] font-semibold text-muted-foreground">
            子代理 ({view.children.length})
          </div>
          <ul className="flex flex-col gap-0.5">
            {view.children.map((c) => (
              <li key={c.agent_id}>
                <button
                  type="button"
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-[12px] hover:bg-accent"
                  onClick={() =>
                    openSubagentTab({
                      nodeId: c.agent_id,
                      label: c.label || c.agent_id,
                      rootSessionId,
                    })
                  }
                >
                  <Network size={12} className="shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate">{c.label || c.agent_id}</span>
                  <span className={cn("shrink-0", statusColor(c.status))}>{c.status}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Full transcript (best-effort; root-direct children only) */}
      {targetCap && (
        <div className="rounded-md border border-border bg-card">
          <div className="flex items-center gap-2 px-3 py-2">
            <Button variant="ghost" size="sm" onClick={() => void loadTranscript()} disabled={transcriptLoading}>
              {transcriptLoading ? "加载中…" : "查看完整 transcript"}
            </Button>
            {transcriptError && (
              <span className="text-[12px] text-danger">{transcriptError}</span>
            )}
          </div>
          {transcript != null && (
            <pre className="max-h-96 overflow-auto border-t border-border px-3 py-2 text-[11px] whitespace-pre-wrap">
              {typeof transcript === "string"
                ? transcript
                : JSON.stringify(transcript, null, 2)}
            </pre>
          )}
        </div>
      )}
      <div ref={bottomRef} />
    </div>
  );
}

/** Read-only rendering of a subagent's captured ChatMessages. */
function SubagentMessages({
  messages,
  textSnapshot,
}: {
  messages: ChatMessage[];
  textSnapshot?: string | null;
}) {
  if (messages.length === 0) {
    if (textSnapshot) {
      return (
        <div className="rounded-md border border-border bg-card p-3 text-[13px] whitespace-pre-wrap">
          {textSnapshot}
        </div>
      );
    }
    return <div className="text-[12px] text-muted-foreground">暂无消息。</div>;
  }

  // Index role:"tool" results by tool_call_id so each tool call renders as one
  // collapsed ToolCallCard (call + result paired) instead of flat chips; the
  // tool messages themselves are skipped — they are shown inside their card.
  const toolResults = new Map<string, ChatMessage>();
  for (const m of messages) {
    if (m.role === "tool" && m.tool_call_id) toolResults.set(m.tool_call_id, m);
  }

  return (
    <div className="flex flex-col gap-2">
      {messages.map((m, i) => (
        m.role === "tool" ? null : (
          <div
            key={i}
            className={cn(
              "flex flex-col gap-1 rounded-lg px-3 py-2",
              m.role === "user" ? "bg-primary/10 items-end" : "bg-card",
            )}
          >
            <div className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
              <span
                className={cn(
                  "h-1.5 w-1.5 rounded-full",
                  m.role === "assistant" ? "bg-primary" : "bg-muted-foreground",
                )}
              />
              {m.role}
            </div>
            {m.content && (
              <div className={cn("text-[13px]", m.role === "user" && "max-w-[85%]")}>
                {m.role === "assistant" ? (
                  <Markdown>{m.content}</Markdown>
                ) : (
                  <span className="whitespace-pre-wrap">{m.content}</span>
                )}
              </div>
            )}
            {m.tool_calls && m.tool_calls.length > 0 && (
              <div className="flex w-full flex-col gap-1">
                {m.tool_calls.map((tc) => {
                  const result = toolResults.get(tc.id);
                  return result ? (
                    <ToolCallCard
                      key={tc.id}
                      exec={{
                        call: tc,
                        response: { success: true, content: result.content },
                      }}
                    />
                  ) : (
                    <RunningToolCard
                      key={tc.id}
                      name={tc.function.name}
                      args={parseArgs(tc.function.arguments)}
                    />
                  );
                })}
              </div>
            )}
          </div>
        )
      ))}
    </div>
  );
}

/** Best-effort parse of a tool call's JSON argument string. */
function parseArgs(argsJson: string): Record<string, unknown> {
  try {
    return JSON.parse(argsJson) as Record<string, unknown>;
  } catch {
    return {};
  }
}

/** Compact GFM markdown renderer for read-only subagent output. */
function Markdown({ children }: { children: string }) {
  return (
    <div
      className={cn(
        "leading-relaxed",
        "[&_p]:my-1.5",
        "[&_ul]:my-1 [&_ul]:list-disc [&_ul]:pl-5",
        "[&_ol]:my-1 [&_ol]:list-decimal [&_ol]:pl-5",
        "[&_li]:my-0.5",
        "[&_code]:rounded [&_code]:bg-background [&_code]:px-1 [&_code]:font-mono [&_code]:text-[0.85em]",
        "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border [&_pre]:bg-background [&_pre]:px-3 [&_pre]:py-2",
      )}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  );
}
