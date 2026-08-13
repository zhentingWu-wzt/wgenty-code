import { Monitor, Moon, PanelLeft, PanelRight, Sun } from "lucide-react";
import { useUiStore } from "../../state/uiStore";
import type { ThemeMode } from "../../lib/theme";
import { Button } from "../ui/button";
import * as DropdownMenu from "../ui/dropdown-menu";

const THEME_ITEMS: { value: ThemeMode; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
  { value: "system", label: "System", icon: Monitor },
];

/** 顶部应用栏：品牌 + 左栏开关 · 主题切换 + 右栏开关。不仿 macOS 窗口按钮。 */
export function AppTopbar() {
  const theme = useUiStore((s) => s.theme);
  const setTheme = useUiStore((s) => s.setTheme);
  const toggleLeft = useUiStore((s) => s.toggleLeft);
  const rightPanel = useUiStore((s) => s.rightPanel);
  const setRightPanel = useUiStore((s) => s.setRightPanel);

  return (
    <header className="flex h-10 shrink-0 items-center gap-1 border-b border-border bg-background px-2">
      <Button variant="ghost" size="icon" onClick={toggleLeft} title="Toggle sidebar">
        <PanelLeft size={15} />
      </Button>
      <span className="ml-1 text-[13px] font-semibold">wgenty-code</span>
      <div className="flex-1" />
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <Button variant="ghost" size="icon" title="Theme">
            {theme === "dark" ? <Moon size={15} /> : theme === "light" ? <Sun size={15} /> : <Monitor size={15} />}
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="end">
          {THEME_ITEMS.map(({ value, label, icon: Icon }) => (
            <DropdownMenu.Item key={value} onSelect={() => setTheme(value)}>
              <Icon size={13} />
              {label}
              {theme === value && <span className="ml-auto text-muted-foreground">✓</span>}
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Root>
      <Button
        variant="ghost"
        size="icon"
        onClick={() => setRightPanel(rightPanel === null ? "sessions" : null)}
        title="Toggle right panel"
      >
        <PanelRight size={15} />
      </Button>
    </header>
  );
}
