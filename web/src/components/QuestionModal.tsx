import { useState } from "react";
import type { DaemonClient } from "../api/client";
import { useSessionStore } from "../state/sessionContext";

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

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal question-modal">
        <div className="modal-title">Question</div>
        <p className="question-text">{question.question}</p>
        <div className="question-options">
          {question.options.map((opt) => (
            <button
              key={opt.label}
              type="button"
              className={`question-option ${selected === opt.label ? "selected" : ""}`}
              onClick={() => setSelected(opt.label)}
              disabled={submitting}
            >
              <span className="question-option-label">{opt.label}</span>
              <span className="question-option-desc">{opt.description}</span>
            </button>
          ))}
        </div>
        <div className="question-other">
          <input
            className="question-other-input"
            placeholder="Other…"
            value={otherText}
            onChange={(e) => setOtherText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && onOther()}
            disabled={submitting}
          />
        </div>
        <div className="modal-actions">
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => selected && submit(selected)}
            disabled={!selected || submitting}
          >
            Submit
          </button>
        </div>
      </div>
    </div>
  );
}
