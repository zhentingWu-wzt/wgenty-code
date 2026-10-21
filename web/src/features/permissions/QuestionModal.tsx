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
  const pendingPermission = useSessionStore((s) => s.pendingPermission);
  const clearQuestion = useSessionStore((s) => s.clearQuestion);
  // Multi-select aware selection (question.multi_select): an array either
  // way — single-select keeps at most one element.
  const [selected, setSelected] = useState<string[]>([]);
  const [otherText, setOtherText] = useState("");
  const [submitting, setSubmitting] = useState(false);

  // Permission prompts block tool execution and take precedence (见文件头
  // 注释)：权限待处理时不渲染问题卡，两者不会同时堆叠挤压聊天区。
  if (!question || pendingPermission) return null;

  const submit = async (answers: string[]) => {
    setSubmitting(true);
    try {
      await client.resolveInteraction(
        question.request_id,
        JSON.stringify({ selected: answers }),
      );
    } catch {
      // Best-effort; the bridge returns a default if the waiter drops.
    } finally {
      clearQuestion();
      setSelected([]);
      setOtherText("");
      setSubmitting(false);
    }
  };

  // Toggle one option: multi-select toggles membership; single-select
  // replaces (clicking the active option clears it).
  const toggle = (label: string) => {
    setSelected((s) =>
      s.includes(label)
        ? s.filter((l) => l !== label)
        : question.multi_select
          ? [...s, label]
          : [label],
    );
  };

  // Submit accepts picked options and/or the free-text "Other" answer —
  // a typed answer alone (no option clicked) must be submittable. In
  // multi-select mode the text answer is appended to the selection.
  const submitAll = () => {
    const other = otherText.trim();
    if (question.multi_select) {
      const answers = [...selected, ...(other ? [other] : [])];
      if (answers.length > 0) submit(answers);
    } else if (selected[0]) {
      submit([selected[0]]);
    } else if (other) {
      submit([other]);
    }
  };

  // 布局内嵌横幅（App.tsx 中置于聊天区与输入框之间）：不遮挡聊天内容，
  // 选项过多时卡片内部滚动（max-h-[50dvh] 防止过度压缩聊天区）。
  return (
    <div
      role="dialog"
      aria-label="Question"
      className="flex max-h-[50dvh] shrink-0 flex-col overflow-y-auto border-t border-primary/40 bg-popover px-3 py-2.5"
    >
      <div className="mb-1.5 flex items-center gap-1.5 text-[13px] font-semibold">
        <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-primary" />
        Question
        <span className="text-[11px] font-normal text-muted-foreground">
          {question.multi_select ? "· select all that apply" : "· select one"}
        </span>
      </div>
      <p className="mb-2 leading-relaxed">{question.question}</p>
      <div className="mb-2 flex flex-col gap-1.5">
        {question.options.map((opt) => {
          const isSelected = selected.includes(opt.label);
          return (
            <button
              key={opt.label}
              type="button"
              className={cn(
                "flex flex-col gap-0.5 rounded-md border px-3 py-2 text-left",
                isSelected
                  ? "border-foreground bg-accent"
                  : "border-border bg-background hover:bg-accent",
              )}
              onClick={() => toggle(opt.label)}
              disabled={submitting}
            >
              <span className="flex w-full items-center justify-between gap-2">
                <span className="text-[13px] font-medium">{opt.label}</span>
                {/* Checkbox affordance: signals toggling is expected, and
                    distinguishes multi-select from radio-like single-select. */}
                <span
                  className={cn(
                    "flex h-4 w-4 shrink-0 items-center justify-center rounded-sm border text-[10px]",
                    isSelected
                      ? "border-primary bg-primary text-primary-foreground"
                      : "border-border",
                  )}
                >
                  {isSelected && "✓"}
                </span>
              </span>
              <span className="text-[12px] text-muted-foreground">{opt.description}</span>
            </button>
          );
        })}
      </div>
      <div className="mb-2">
        <input
          className="w-full rounded-md border border-input bg-background px-2.5 py-1.5 text-[13px] outline-none focus:border-ring"
          placeholder="Other…"
          value={otherText}
          onChange={(e) => setOtherText(e.target.value)}
          // IME guard (same as Composer): while composing (Chinese pinyin
          // etc.) Enter confirms the candidate, not the submission.
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Enter") submitAll();
          }}
          disabled={submitting}
        />
      </div>
      <div className="flex justify-end gap-2">
        <Button onClick={submitAll} disabled={(selected.length === 0 && !otherText.trim()) || submitting}>
          Submit
        </Button>
      </div>
    </div>
  );
}

