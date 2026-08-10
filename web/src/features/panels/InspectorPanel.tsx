import { useState } from "react";
import { useSessionManager } from "../../state/sessionManager";
import type {
  TurnContextData,
  TurnContextLayer,
  TurnContextMemory,
  TurnContextMessage,
} from "../../state/sessionStore";

/**
 * Inspector perspective panel — shows turn-context data from the most recent
 * daemon turn: system prompt layers, recalled memories, new messages, hook
 * reminder, and token usage.
 *
 * Data arrives via the `turn_context` SSE event (broadcast after each turn's
 * final save). The panel reads the active session's `turnContext` from its
 * store. When null (no turn completed yet), shows a placeholder.
 */
export function InspectorPanel() {
  const activeStore = useSessionManager((s) => (s.activeId ? s.entries[s.activeId]?.store : null));
  const turnContext = activeStore?.getState().turnContext ?? null;

  if (!turnContext) {
    return (
      <div className="p-2">
        <div className="p-2 text-[12px] text-muted-foreground">
          No turn context yet. Complete a turn to see inspector data.
        </div>
      </div>
    );
  }

  return <TurnContextView data={turnContext} />;
}

function TurnContextView({ data }: { data: TurnContextData }) {
  const [tab, setTab] = useState<"layers" | "memories" | "messages" | "hooks" | "tokens">("layers");

  const tabs = [
    { id: "layers" as const, label: `Layers (${data.layers.length})` },
    { id: "memories" as const, label: `Memories (${data.recalled_memories.length})` },
    { id: "messages" as const, label: `Messages (${data.new_messages.length})` },
    { id: "hooks" as const, label: "Hooks" },
    { id: "tokens" as const, label: "Tokens" },
  ];

  return (
    <div className="flex flex-col gap-2 p-2">
      {/* Tab bar */}
      <div className="flex flex-wrap gap-1 border-b border-border pb-1">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            className={
              "rounded-sm px-2 py-0.5 text-[11px] " +
              (tab === t.id
                ? "bg-sidebar-accent text-foreground"
                : "text-muted-foreground hover:bg-accent")
            }
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {tab === "layers" && <LayersTab layers={data.layers} />}
      {tab === "memories" && <MemoriesTab memories={data.recalled_memories} />}
      {tab === "messages" && <MessagesTab messages={data.new_messages} />}
      {tab === "hooks" && <HooksTab reminder={data.reminder} />}
      {tab === "tokens" && <TokensTab usage={data.usage} />}
    </div>
  );
}

function LayersTab({ layers }: { layers: TurnContextLayer[] }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  if (layers.length === 0) {
    return <Empty msg="No system prompt layers" />;
  }
  return (
    <ul className="flex flex-col gap-0.5">
      {layers.map((layer) => (
        <li key={layer.label} className="rounded-sm border border-border p-1.5">
          <button
            type="button"
            className="flex w-full items-center gap-2 text-left"
            onClick={() => setExpanded(expanded === layer.label ? null : layer.label)}
          >
            <span className="min-w-0 flex-1 truncate text-[12px]">{layer.label}</span>
            <span className="shrink-0 text-[10px] text-muted-foreground">{layer.char_count} chars</span>
          </button>
          {expanded === layer.label && (
            <div className="mt-1 text-[10px] text-muted-foreground">Source: {layer.source}</div>
          )}
        </li>
      ))}
    </ul>
  );
}

function MemoriesTab({ memories }: { memories: TurnContextMemory[] }) {
  if (memories.length === 0) {
    return <Empty msg="No memories recalled this turn" />;
  }
  return (
    <ul className="flex flex-col gap-0.5">
      {memories.map((m, i) => (
        <li key={i} className="rounded-sm border border-border p-1.5">
          <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
            <span className="text-primary">{m.memory_type}</span>
            <span className="ml-auto" title="importance">{m.importance.toFixed(2)}</span>
          </div>
          <div className="pt-0.5 text-[12px]">{m.content_preview}</div>
        </li>
      ))}
    </ul>
  );
}

function MessagesTab({ messages }: { messages: TurnContextMessage[] }) {
  if (messages.length === 0) {
    return <Empty msg="No new messages this turn" />;
  }
  return (
    <ul className="flex flex-col gap-0.5">
      {messages.map((m, i) => (
        <li key={i} className="rounded-sm border border-border p-1.5">
          <span className="text-[10px] text-primary">{m.role}</span>
          <div className="pt-0.5 text-[12px] line-clamp-3">{m.content}</div>
        </li>
      ))}
    </ul>
  );
}

function HooksTab({ reminder }: { reminder: TurnContextData["reminder"] }) {
  if (!reminder) {
    return (
      <Empty msg="No hook injections this turn. (Daemon hook reminder integration is pending — see inspector-perspective change notes.)" />
    );
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="rounded-sm border border-border p-1.5">
        <div className="text-[10px] text-muted-foreground">to_model</div>
        <pre className="whitespace-pre-wrap pt-0.5 text-[11px]">{reminder.to_model}</pre>
      </div>
      {reminder.to_transcript && (
        <div className="rounded-sm border border-border p-1.5">
          <div className="text-[10px] text-muted-foreground">to_transcript</div>
          <pre className="whitespace-pre-wrap pt-0.5 text-[11px]">{reminder.to_transcript}</pre>
        </div>
      )}
    </div>
  );
}

function TokensTab({ usage }: { usage: TurnContextData["usage"] }) {
  return (
    <div className="flex gap-4 p-1">
      <Metric label="Prompt" value={usage.prompt_tokens} />
      <Metric label="Completion" value={usage.completion_tokens} />
      <Metric label="Total" value={usage.total_tokens} />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col">
      <span className="text-[15px] font-semibold">{value.toLocaleString()}</span>
      <span className="text-[11px] text-muted-foreground">{label}</span>
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return <div className="p-2 text-[12px] text-muted-foreground">{msg}</div>;
}
