import { useState } from "react";
import type { DaemonClient } from "../../api/client";
import { useSessionStore } from "../../state/sessionContext";
import { cn } from "../../lib/utils";
import { Button } from "../../components/ui/button";

/**
 * Modal for ask_user_question prompts pushed from the server-side loop via
 * trace SSE. Renders the question + options as clickable rows (+ an implicit
 * "Other" free-text). On submit, POSTs the answer to /interactions/:id/resolve
 * and clears the prompt.
 *
 * Lower priority than PermissionModal (permission prompts block tool
 * execution; questions are mid-loop). Rendered only when no permission prompt
 * is pending.
 */
export function QuestionModal({ client }: { client: DaemonClient }) {
  const question = useSessionStore((s) => s.pendingQuestion);
  const clearQuestion = useSessionStore((s) => s.clearQuestion);
  const [selected, setSelected] = useState<string | null>(null);
  const [otherText, setOtherText] = useState("");
  const [submitting, setSubmitting] = useState(false);

  if (!question) return null;

  const submit = async (answer: string) => {
    setSubmitting(true);
    try {
      await client.resolveInteraction(
        question.request_id,
        JSON.stringify({ selected: [answer] }),
      );
    } catch {
      // Best-effort; the bridge returns a default if the waiter drops.
    } finally {
      clearQuestion();
      setSelected(null);
      setOtherText("");
      setSubmitting(false);
    }
  };

  const onOther = () => {
    if (otherText.trim()) submit(otherText.trim());
  };

  // 底部停靠卡片而非全屏遮罩弹窗（与 PermissionModal 一致）：问题上下文
  // 保持可见，选项过多时卡片内部滚动（max-h-[70dvh]）。
  return (
    <div
      role="dialog"
      aria-label="Question"
      className="fixed bottom-8 left-1/2 z-50 flex max-h-[70dvh] w-[520px] max-w-[calc(100%-16px)] -translate-x-1/2 flex-col overflow-y-auto rounded-lg border border-primary/40 bg-popover p-4 shadow-2xl"
    >
      <div className="mb-2 flex items-center gap-1.5 text-[15px] font-semibold">
        <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-primary" />
        Question
      </div>
      <p className="mb-3 text-[13px] leading-relaxed">{question.question}</p>
      <div className="mb-2 flex flex-col gap-1.5">
        {question.options.map((opt) => (
          <button
            key={opt.label}
            type="button"
            className={cn(
              "flex flex-col gap-0.5 rounded-md border px-3 py-2 text-left",
              selected === opt.label
                ? "border-foreground bg-accent"
                : "border-border bg-background hover:bg-accent",
            )}
            onClick={() => setSelected(opt.label)}
            disabled={submitting}
          >
            <span className="text-[13px] font-medium">{opt.label}</span>
            <span className="text-[12px] text-muted-foreground">{opt.description}</span>
          </button>
        ))}
      </div>
      <div className="mb-2">
        <input
          className="w-full rounded-md border border-input bg-background px-2.5 py-1.5 text-[13px] outline-none focus:border-ring"
          placeholder="Other…"
          value={otherText}
          onChange={(e) => setOtherText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onOther()}
          disabled={submitting}
        />
      </div>
      <div className="flex justify-end gap-2">
        <Button onClick={() => selected && submit(selected)} disabled={!selected || submitting}>
          Submit
        </Button>
      </div>
    </div>
  );
}

