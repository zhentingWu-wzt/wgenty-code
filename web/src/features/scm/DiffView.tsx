import { useEffect, useMemo, useState } from "react";
import { FileMinus, Loader2, RefreshCw } from "lucide-react";
import { type DaemonClient } from "../../api/client";
import type { FileDiff as FileDiffDto } from "../../api/types";
import type { DiffTabMeta } from "../../state/uiStore";
import { cn } from "../../lib/utils";

/**
 * Diff tab content (`diff:<absPath>`) — the file's full inline diff vs HEAD.
 * The daemon diffs with a huge `-U` context, so `lines` already IS the whole
 * file: context rows carry both line numbers, added rows show green with the
 * new number, deleted rows red with the old number. That satisfies "整个文件
 * + 变更高亮" in one scroll (VSCode inline-diff style).
 */

type LoadState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "done"; diff: FileDiffDto };

/** Row background + sign color per kind. */
const ROW_STYLE = {
  context: "",
  add: "bg-success/15",
  delete: "bg-danger/15",
} as const;

const SIGN = { context: " ", add: "+", delete: "−" } as const;

const STATUS_LABEL: Record<FileDiffDto["status"], string> = {
  modified: "修改",
  added: "新增",
  deleted: "删除",
};

export function DiffView({ meta, client }: { meta: DiffTabMeta; client: DaemonClient }) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  // Bumped by the retry/refresh button to re-run the fetch effect.
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    client.gitDiff(meta.absPath).then(
      (diff) => {
        if (live) setState({ status: "done", diff });
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

  const counts = useMemo(() => {
    if (state.status !== "done") return { added: 0, deleted: 0 };
    let added = 0;
    let deleted = 0;
    for (const l of state.diff.lines) {
      if (l.kind === "add") added += 1;
      else if (l.kind === "delete") deleted += 1;
    }
    return { added, deleted };
  }, [state]);

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="diff-view">
      {/* Header: path + status + counts + refresh. */}
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-3 text-[12px]">
        <FileMinus size={13} className="shrink-0 text-primary" />
        <span className="truncate font-medium" title={meta.absPath}>
          {meta.relPath}
        </span>
        {state.status === "done" && (
          <>
            <span className="shrink-0 text-muted-foreground">
              {STATUS_LABEL[state.diff.status]}
            </span>
            <span className="shrink-0 text-success">+{counts.added}</span>
            <span className="shrink-0 text-danger">−{counts.deleted}</span>
            {state.diff.truncated && (
              <span className="shrink-0 text-[11px] text-muted-foreground">已截断</span>
            )}
          </>
        )}
        <button
          type="button"
          title="Refresh"
          className="ml-auto shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={() => {
            setState({ status: "loading" });
            setAttempt((k) => k + 1);
          }}
        >
          <RefreshCw size={12} />
        </button>
      </div>

      {state.status === "loading" && (
        <div className="flex flex-1 items-center justify-center gap-2 text-muted-foreground">
          <Loader2 size={14} className="animate-spin" />
          <span className="text-xs">加载 diff…</span>
        </div>
      )}

      {state.status === "error" && (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
          <span className="text-xs text-danger">{state.message}</span>
          <button
            type="button"
            className="rounded-md border border-border px-2 py-1 text-xs hover:bg-accent"
            onClick={() => {
              setState({ status: "loading" });
              setAttempt((k) => k + 1);
            }}
          >
            重试
          </button>
        </div>
      )}

      {state.status === "done" && state.diff.lines.length === 0 && (
        <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">
          没有差异(文件与 HEAD 一致)
        </div>
      )}

      {state.status === "done" && state.diff.lines.length > 0 && (
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="min-w-max font-mono text-[12px] leading-5">
            {state.diff.lines.map((l, i) => (
              <div key={i} className={cn("flex", ROW_STYLE[l.kind])}>
                <span className="w-10 shrink-0 select-none pr-1 text-right text-[11px] text-muted-foreground/70">
                  {l.old_no ?? ""}
                </span>
                <span className="w-10 shrink-0 select-none pr-1 text-right text-[11px] text-muted-foreground/70">
                  {l.new_no ?? ""}
                </span>
                <span
                  className={cn(
                    "w-4 shrink-0 select-none text-center",
                    l.kind === "add" && "text-success",
                    l.kind === "delete" && "text-danger",
                  )}
                >
                  {SIGN[l.kind]}
                </span>
                <span className="whitespace-pre pr-4">{l.text}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
