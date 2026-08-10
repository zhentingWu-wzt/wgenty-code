import { useDisplayPrefs, type DisplayMode } from "../../state/displayPrefs";
import { cn } from "../../lib/utils";

const MODES: { value: DisplayMode; label: string; title: string }[] = [
  {
    value: "single",
    label: "单气泡",
    title: "整个 turn 一个气泡：所有文字在上、工具卡片全部在下",
  },
  {
    value: "rounds",
    label: "按轮次",
    title: "每轮 LLM 响应一个气泡：该轮文字 + 该轮工具卡片",
  },
  {
    value: "timeline",
    label: "时间线",
    title: "工具作为独立条目按到达顺序穿插在文字之间（与 TUI 一致，执行中有 running 占位）",
  },
];

/** Chat 展示模式切换：单气泡 / 按轮次 / 时间线。全局生效并持久化到 localStorage。 */
export function DisplayModeToggle() {
  const mode = useDisplayPrefs((s) => s.mode);
  const setMode = useDisplayPrefs((s) => s.setMode);

  return (
    <div className="flex shrink-0 items-center gap-0.5 px-2">
      <div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
        {MODES.map((m) => (
          <button
            key={m.value}
            type="button"
            title={m.title}
            onClick={() => setMode(m.value)}
            className={cn(
              "rounded px-2 py-0.5 text-[11px] transition-colors",
              mode === m.value
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-accent",
            )}
          >
            {m.label}
          </button>
        ))}
      </div>
    </div>
  );
}
