import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";

interface CommandModalProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
}

/**
 * Generic shell for slash-command panels (/model, /memory, /undo, …).
 * Backdrop click or Esc closes; content is scrollable. Visual language mirrors
 * the permission modal (.modal-backdrop/.modal in styles.css).
 */
export function CommandModal({ title, onClose, children }: CommandModalProps) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal command-modal"
        role="dialog"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="command-modal-head">
          <span className="command-modal-title">{title}</span>
          <button type="button" className="command-modal-close" onClick={onClose} title="Close">
            <X size={14} />
          </button>
        </div>
        <div className="command-modal-body">{children}</div>
      </div>
    </div>
  );
}
