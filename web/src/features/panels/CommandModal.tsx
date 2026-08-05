import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";
import { Button } from "../../components/ui/button";

interface CommandModalProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
}

/**
 * Generic shell for slash-command panels (/model, /memory, /undo, …).
 * Backdrop click or Esc closes; content is scrollable. Visual language mirrors
 * the permission modal (fixed backdrop + popover card).
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
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="flex max-h-[70vh] w-[560px] max-w-[90%] flex-col rounded-lg border border-border bg-popover"
        role="dialog"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-border px-4 py-2.5">
          <span className="text-[13px] font-semibold">{title}</span>
          <Button variant="ghost" size="icon" onClick={onClose} title="Close">
            <X size={14} />
          </Button>
        </div>
        <div className="overflow-y-auto px-4 py-3">{children}</div>
      </div>
    </div>
  );
}
