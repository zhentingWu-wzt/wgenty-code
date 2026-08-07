import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Button } from "./button";

export interface ConfirmOptions {
  title: string;
  message: ReactNode;
  /** Confirm button label (defaults to "Confirm"). */
  confirmLabel?: string;
  /** Cancel button label (defaults to "Cancel"). */
  cancelLabel?: string;
  /** Red destructive confirm button + alertdialog semantics. */
  destructive?: boolean;
}

type ConfirmFn = (options: ConfirmOptions) => Promise<boolean>;

const ConfirmContext = createContext<ConfirmFn | null>(null);

/**
 * Imperative confirmation dialog hook. Resolves `true` on confirm, `false` on
 * cancel (backdrop click, Esc, or Cancel button). Drop-in for the control
 * flow of the old `if (!window.confirm(...)) return;` pattern:
 *
 *   const confirm = useConfirm();
 *   if (!(await confirm({ title: "...", message: "...", destructive: true }))) return;
 *
 * Must be rendered inside <ConfirmProvider>.
 */
export function useConfirm(): ConfirmFn {
  const fn = useContext(ConfirmContext);
  if (!fn) throw new Error("useConfirm must be used within <ConfirmProvider>");
  return fn;
}

/**
 * Provides the confirm dialog. Mount once near the app root; the rendered
 * dialog only appears while a confirm() call is pending.
 */
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [options, setOptions] = useState<ConfirmOptions | null>(null);
  const resolverRef = useRef<((value: boolean) => void) | null>(null);

  const confirm = useCallback<ConfirmFn>(
    (opts) =>
      new Promise<boolean>((resolve) => {
        resolverRef.current = resolve;
        setOptions(opts);
      }),
    [],
  );

  const close = useCallback((result: boolean) => {
    resolverRef.current?.(result);
    resolverRef.current = null;
    setOptions(null);
  }, []);

  useEffect(() => {
    if (!options) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [options, close]);

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      {options && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
          onClick={() => close(false)}
        >
          <div
            className="flex w-[420px] max-w-[90%] flex-col gap-3 rounded-lg border border-border bg-popover p-4 shadow-xl"
            role="alertdialog"
            aria-label={options.title}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="text-[13px] font-semibold">{options.title}</div>
            <div className="text-[13px] text-muted-foreground">{options.message}</div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => close(false)}>
                {options.cancelLabel ?? "Cancel"}
              </Button>
              <Button
                variant={options.destructive ? "destructive" : "default"}
                size="sm"
                onClick={() => close(true)}
              >
                {options.confirmLabel ?? "Confirm"}
              </Button>
            </div>
          </div>
        </div>
      )}
    </ConfirmContext.Provider>
  );
}
