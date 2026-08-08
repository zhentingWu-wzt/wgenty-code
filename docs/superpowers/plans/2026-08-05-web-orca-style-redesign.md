# Web 端 Orca 风格重设计 · 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `web/` 前端改造成 Orca 桌面端风格的三段式工作台（顶栏 + 左侧会话树 + tab 栏 + 右侧 activity bar 面板 + 底部状态栏），引入 Tailwind v4 + shadcn 风格组件与 light/dark 主题。

**Architecture:** 新壳渐进迁移（设计文档 `docs/superpowers/specs/2026-08-05-web-orca-style-redesign-design.md`）。通信层（`web/src/api/`、`web/src/agent/sessionRunner.ts`）与会话状态层（`state/sessionManager.ts`、`state/sessionStore.ts`）零改动；新增 `state/uiStore.ts` 管理布局/tab/主题；样式从单一 `styles.css`（1767 行，仅暗色）迁移到 Tailwind + token 体系。

**Tech Stack:** React 18 + Vite 5 + zustand 4 + Tailwind CSS v4（`@tailwindcss/vite`）+ radix-ui + lucide-react + vitest/jsdom。

## Global Constraints

- 不新增任何 daemon 后端能力；只用现有 REST + SSE API。
- 不复制 Orca 的代码与资源（GPL）；token 数值、组件代码均自行编写。
- 每个 Task 结束：`pnpm --dir web test`、`pnpm --dir web typecheck`、`pnpm --dir web build` 必须全绿。
- 提交信息遵循 Conventional Commits，scope 用 `web`（如 `feat(web): ...`）。
- zustand 为 v4（`create<T>()(...)` 用法，subscribe 回调签名为 `(state, prevState)`）。
- Tailwind v4：无 `tailwind.config.js`，全部配置在 CSS 内（`@import "tailwindcss"` + `@theme`）。
- 新旧样式并存期：旧 `styles.css` 保持可用直到 Task 7 删除；新组件一律用 Tailwind 类，不再向 `styles.css` 追加规则。

---

### Task 1: Tailwind + shadcn 基础设施

**Files:**
- Modify: `web/package.json`（通过 pnpm add）
- Modify: `web/vite.config.ts`（plugins 数组，第 31 行）
- Create: `web/src/styles/globals.css`
- Create: `web/src/lib/utils.ts`
- Create: `web/src/lib/utils.test.ts`
- Modify: `web/src/main.tsx:6`（追加 globals.css 引入）

**Interfaces:**
- Produces: `cn(...inputs: ClassValue[]): string`（`web/src/lib/utils.ts`）——后续所有组件使用。
- Produces: `web/src/styles/globals.css` 的 token 变量（`--background` 等）与 Tailwind 主题映射——后续组件用 `bg-background`、`text-muted-foreground` 等类。
- Produces: 额外状态色类 `text-success` / `text-warning` / `text-danger` / `bg-success` 等。

- [ ] **Step 1: 安装依赖**

```bash
pnpm --dir web add tailwindcss@^4 @tailwindcss/vite@^4 clsx tailwind-merge class-variance-authority radix-ui
```

- [ ] **Step 2: 写 cn() 的失败测试**

Create `web/src/lib/utils.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { cn } from "./utils";

describe("cn", () => {
  it("merges conditional classes", () => {
    expect(cn("px-2", false && "hidden", "text-sm")).toBe("px-2 text-sm");
  });

  it("tailwind-merge dedupes conflicting utilities (last wins)", () => {
    expect(cn("px-2", "px-4")).toBe("px-4");
  });
});
```

Run: `pnpm --dir web vitest run src/lib/utils.test.ts`
Expected: FAIL（`Cannot find module './utils'`）

- [ ] **Step 3: 实现 cn()**

Create `web/src/lib/utils.ts`:

```ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** clsx + tailwind-merge：shadcn 风格的类名合并工具。 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
```

Run: `pnpm --dir web vitest run src/lib/utils.test.ts`
Expected: PASS

- [ ] **Step 4: 配置 Tailwind vite 插件**

Modify `web/vite.config.ts`：在 `plugins` 数组中加入 `tailwindcss()`：

```ts
import tailwindcss from "@tailwindcss/vite";
// ...
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // 其余不变
```

- [ ] **Step 5: 写 globals.css token 体系**

Create `web/src/styles/globals.css`（完整内容，直接写入）：

```css
@import "tailwindcss";

@custom-variant dark (&:is(.dark *));

/*
 * shadcn 风格 token 体系（neutral 灰阶 + 品牌蓝 accent）。
 * 暗色值沿用旧 styles.css 的 Codex 风格调色板；亮色为新增。
 */
:root {
  --background: #ffffff;
  --foreground: #1a1a1a;
  --card: #ffffff;
  --card-foreground: #1a1a1a;
  --popover: #ffffff;
  --popover-foreground: #1a1a1a;
  --primary: #3d63dd;
  --primary-foreground: #ffffff;
  --secondary: #f4f4f5;
  --secondary-foreground: #1a1a1a;
  --muted: #f4f4f5;
  --muted-foreground: #737373;
  --accent: #f4f4f5;
  --accent-foreground: #1a1a1a;
  --destructive: #dc2626;
  --destructive-foreground: #ffffff;
  --border: #e5e5e5;
  --input: #e5e5e5;
  --ring: #3d63dd;
  --success: #16a34a;
  --warning: #d97706;
  --danger: #dc2626;
  --sidebar: #fafafa;
  --sidebar-foreground: #1a1a1a;
  --sidebar-border: #e5e5e5;
  --sidebar-accent: #f4f4f5;
  --sidebar-accent-foreground: #1a1a1a;
  --radius: 0.375rem;
}

.dark {
  --background: #0d0d0d;
  --foreground: #ededed;
  --card: #161616;
  --card-foreground: #ededed;
  --popover: #1f1f1f;
  --popover-foreground: #ededed;
  --primary: #6e8efb;
  --primary-foreground: #0d0d0d;
  --secondary: #1f1f1f;
  --secondary-foreground: #ededed;
  --muted: #1f1f1f;
  --muted-foreground: #8a8a8a;
  --accent: #262626;
  --accent-foreground: #ededed;
  --destructive: #e5606b;
  --destructive-foreground: #0d0d0d;
  --border: #262626;
  --input: #333333;
  --ring: #6e8efb;
  --success: #7ec880;
  --warning: #d9a441;
  --danger: #e5606b;
  --sidebar: #111111;
  --sidebar-foreground: #ededed;
  --sidebar-border: #262626;
  --sidebar-accent: #1f1f1f;
  --sidebar-accent-foreground: #ededed;
}

@theme inline {
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  --color-card: var(--card);
  --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover);
  --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary);
  --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary);
  --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted);
  --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent);
  --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive);
  --color-destructive-foreground: var(--destructive-foreground);
  --color-border: var(--border);
  --color-input: var(--input);
  --color-ring: var(--ring);
  --color-success: var(--success);
  --color-warning: var(--warning);
  --color-danger: var(--danger);
  --color-sidebar: var(--sidebar);
  --color-sidebar-foreground: var(--sidebar-foreground);
  --color-sidebar-border: var(--sidebar-border);
  --color-sidebar-accent: var(--sidebar-accent);
  --color-sidebar-accent-foreground: var(--sidebar-accent-foreground);
  --radius-sm: calc(var(--radius) - 2px);
  --radius-md: var(--radius);
  --radius-lg: calc(var(--radius) + 2px);
  --font-sans: "Inter Variable", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", "SF Mono", ui-monospace, "Menlo", monospace;
}

@layer base {
  * {
    border-color: var(--border);
  }
  body {
    background: var(--background);
    color: var(--foreground);
    font-family: var(--font-sans);
    font-size: 13px;
    font-feature-settings: "cv02", "cv03", "cv04", "cv11";
  }
}
```

- [ ] **Step 6: main.tsx 引入 globals.css**

Modify `web/src/main.tsx:6`，在 `import "./styles.css";` 之后追加一行：

```ts
import "./styles/globals.css";
```

（旧 `styles.css` 在 Task 7 才删除，并存期 globals 在后，token 变量不冲突——旧文件用 `--bg/--fg` 命名，新文件用 `--background/--foreground`。）

- [ ] **Step 7: 验证构建**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿；`pnpm --dir web dev` 打开页面，旧 UI 外观无变化（globals 只定义变量和 base 样式）。

- [ ] **Step 8: Commit**

```bash
git add web/package.json pnpm-lock.yaml web/vite.config.ts web/src/styles/globals.css web/src/lib/ web/src/main.tsx
git commit -m "feat(web): add tailwind v4 + shadcn token infrastructure"
```

---

### Task 2: uiStore + 主题系统

**Files:**
- Create: `web/src/lib/theme.ts`
- Create: `web/src/lib/theme.test.ts`
- Create: `web/src/state/uiStore.ts`
- Create: `web/src/state/uiStore.test.ts`

**Interfaces:**
- Produces: `ThemeMode = "light" | "dark" | "system"`（`lib/theme.ts`）
- Produces: `applyTheme(mode: ThemeMode): void`、`readStoredTheme(): ThemeMode`（`lib/theme.ts`）
- Produces: `RightPanelId = "sessions" | "skills" | "memory" | "checkpoints" | "tasks"`（`state/uiStore.ts`）
- Produces: `useUiStore`：`{ theme, leftCollapsed, rightPanel, setTheme, toggleLeft, setRightPanel, toggleRightPanel }`——Task 3/4/6 消费。

- [ ] **Step 1: 写 theme 的失败测试**

Create `web/src/lib/theme.test.ts`:

```ts
import { beforeEach, describe, expect, it } from "vitest";
import { applyTheme, readStoredTheme } from "./theme";

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark");
  });

  it("defaults to system when nothing stored", () => {
    expect(readStoredTheme()).toBe("system");
  });

  it("applyTheme(dark) adds .dark to documentElement and persists", () => {
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("wgenty-theme")).toBe("dark");
  });

  it("applyTheme(light) removes .dark", () => {
    document.documentElement.classList.add("dark");
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
```

Run: `pnpm --dir web vitest run src/lib/theme.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 2: 实现 lib/theme.ts**

Create `web/src/lib/theme.ts`:

```ts
/** ThemeMode 定义在此处（而非 uiStore），避免 theme.ts ↔ uiStore.ts 循环依赖。 */
export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "wgenty-theme";

export function readStoredTheme(): ThemeMode {
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

export function resolveDark(mode: ThemeMode): boolean {
  return (
    mode === "dark" ||
    (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

/** 切换 documentElement 的 .dark class 并持久化选择。 */
export function applyTheme(mode: ThemeMode): void {
  document.documentElement.classList.toggle("dark", resolveDark(mode));
  localStorage.setItem(STORAGE_KEY, mode);
}
```

Run: `pnpm --dir web vitest run src/lib/theme.test.ts`
Expected: PASS（jsdom 的 matchMedia 默认 matches: false，system 解析为 light）

- [ ] **Step 3: 写 uiStore 的失败测试**

Create `web/src/state/uiStore.test.ts`:

```ts
import { beforeEach, describe, expect, it } from "vitest";
import { useUiStore } from "./uiStore";

describe("uiStore", () => {
  beforeEach(() => {
    useUiStore.setState({ leftCollapsed: false, rightPanel: null });
  });

  it("toggleLeft flips leftCollapsed", () => {
    useUiStore.getState().toggleLeft();
    expect(useUiStore.getState().leftCollapsed).toBe(true);
  });

  it("toggleRightPanel opens then closes the same panel", () => {
    useUiStore.getState().toggleRightPanel("skills");
    expect(useUiStore.getState().rightPanel).toBe("skills");
    useUiStore.getState().toggleRightPanel("skills");
    expect(useUiStore.getState().rightPanel).toBeNull();
  });

  it("toggleRightPanel switches to a different panel", () => {
    useUiStore.getState().toggleRightPanel("skills");
    useUiStore.getState().toggleRightPanel("memory");
    expect(useUiStore.getState().rightPanel).toBe("memory");
  });
});
```

Run: `pnpm --dir web vitest run src/state/uiStore.test.ts`
Expected: FAIL

- [ ] **Step 4: 实现 state/uiStore.ts**

Create `web/src/state/uiStore.ts`:

```ts
/**
 * UI 布局状态：主题、左右栏显隐、右栏当前面板。
 * 只持有 UI 事实，不碰会话数据（会话在 sessionManager）。
 */
import { create } from "zustand";
import { applyTheme, readStoredTheme, type ThemeMode } from "../lib/theme";

export type RightPanelId = "sessions" | "skills" | "memory" | "checkpoints" | "tasks";

interface UiState {
  theme: ThemeMode;
  leftCollapsed: boolean;
  rightPanel: RightPanelId | null;

  setTheme: (t: ThemeMode) => void;
  toggleLeft: () => void;
  setRightPanel: (p: RightPanelId | null) => void;
  /** 点已激活的图标 = 收起右栏；点其他图标 = 切换面板。 */
  toggleRightPanel: (p: RightPanelId) => void;
}

export const useUiStore = create<UiState>((set) => ({
  theme: readStoredTheme(),
  leftCollapsed: false,
  rightPanel: null,

  setTheme: (theme) => {
    applyTheme(theme);
    set({ theme });
  },
  toggleLeft: () => set((s) => ({ leftCollapsed: !s.leftCollapsed })),
  setRightPanel: (rightPanel) => set({ rightPanel }),
  toggleRightPanel: (p) => set((s) => ({ rightPanel: s.rightPanel === p ? null : p })),
}));
```

Run: `pnpm --dir web vitest run src/state/uiStore.test.ts`
Expected: PASS

- [ ] **Step 5: main.tsx 启动时应用主题**

Modify `web/src/main.tsx`：在 `createRoot(...).render(...)` 之前追加：

```ts
import { applyTheme, readStoredTheme } from "./lib/theme";
// ...
applyTheme(readStoredTheme());
```

- [ ] **Step 6: 全量验证 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿

```bash
git add web/src/lib/theme.ts web/src/lib/theme.test.ts web/src/state/uiStore.ts web/src/state/uiStore.test.ts web/src/main.tsx
git commit -m "feat(web): add uiStore and light/dark/system theme system"
```

---

### Task 3: 壳层 — AppTopbar + 底部 StatusBar + App 布局重排

**Files:**
- Create: `web/src/components/ui/button.tsx`
- Create: `web/src/components/ui/dropdown-menu.tsx`
- Create: `web/src/components/layout/AppTopbar.tsx`
- Create: `web/src/components/layout/AppTopbar.test.tsx`
- Modify: `web/src/components/StatusBar.tsx`（整体重写为底部栏）
- Modify: `web/src/App.tsx:118-163`（布局重排）

**Interfaces:**
- Consumes: `useUiStore`（Task 2）、`cn`（Task 1）
- Produces: `<AppTopbar />`（无 props，直连 store）；`<Button variant size>`；`DropdownMenu` 命名导出 `{ Root, Trigger, Content, Item }`
- Produces: 底部 `<StatusBar />`（无 props；不再依赖 SessionStoreContext——见 Step 4 说明）

- [ ] **Step 1: ui/button.tsx（shadcn 标准实现，直接写入）**

Create `web/src/components/ui/button.tsx`:

```tsx
import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 rounded-md text-[13px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        outline: "border border-border bg-transparent hover:bg-accent",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
      },
      size: {
        default: "h-8 px-3",
        sm: "h-7 px-2 text-xs",
        icon: "h-7 w-7",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => (
    <button ref={ref} className={cn(buttonVariants({ variant, size }), className)} {...props} />
  ),
);
Button.displayName = "Button";
```

- [ ] **Step 2: ui/dropdown-menu.tsx（radix 薄封装，直接写入）**

Create `web/src/components/ui/dropdown-menu.tsx`:

```tsx
import { DropdownMenu as RadixDropdownMenu } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../../lib/utils";

export const Root = RadixDropdownMenu.Root;
export const Trigger = RadixDropdownMenu.Trigger;

export function Content({ className, ...props }: ComponentProps<typeof RadixDropdownMenu.Content>) {
  return (
    <RadixDropdownMenu.Portal>
      <RadixDropdownMenu.Content
        sideOffset={4}
        className={cn(
          "z-50 min-w-[8rem] rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md",
          className,
        )}
        {...props}
      />
    </RadixDropdownMenu.Portal>
  );
}

export function Item({
  className,
  children,
  ...props
}: ComponentProps<typeof RadixDropdownMenu.Item> & { children: ReactNode }) {
  return (
    <RadixDropdownMenu.Item
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-[13px] outline-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground",
        className,
      )}
      {...props}
    >
      {children}
    </RadixDropdownMenu.Item>
  );
}
```

- [ ] **Step 3: 写 AppTopbar 的失败测试**

Create `web/src/components/layout/AppTopbar.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { AppTopbar } from "./AppTopbar";
import { useUiStore } from "../../state/uiStore";

describe("AppTopbar", () => {
  beforeEach(() => {
    useUiStore.setState({ theme: "system", leftCollapsed: false, rightPanel: null });
  });

  it("renders brand and toggles left sidebar via uiStore", async () => {
    render(<AppTopbar />);
    expect(screen.getByText("wgenty-code")).toBeInTheDocument();
    await userEvent.click(screen.getByTitle("Toggle sidebar"));
    expect(useUiStore.getState().leftCollapsed).toBe(true);
  });

  it("switches theme via dropdown", async () => {
    render(<AppTopbar />);
    await userEvent.click(screen.getByTitle("Theme"));
    await userEvent.click(await screen.findByText("Dark"));
    expect(useUiStore.getState().theme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("toggles right rail", async () => {
    render(<AppTopbar />);
    await userEvent.click(screen.getByTitle("Toggle right panel"));
    expect(useUiStore.getState().rightPanel).toBe("sessions");
    await userEvent.click(screen.getByTitle("Toggle right panel"));
    expect(useUiStore.getState().rightPanel).toBeNull();
  });
});
```

Run: `pnpm --dir web vitest run src/components/layout/AppTopbar.test.tsx`
Expected: FAIL

- [ ] **Step 4: 实现 AppTopbar**

Create `web/src/components/layout/AppTopbar.tsx`:

```tsx
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
```

Run: `pnpm --dir web vitest run src/components/layout/AppTopbar.test.tsx`
Expected: PASS

- [ ] **Step 5: StatusBar 重写为底部栏**

`StatusBar` 目前用 `useSessionStore`（依赖 SessionStoreContext）读 `isRunning`。改为从 `useSessionManager` 读激活 entry 的 `status`，这样它能放在 Provider 之外、布局更自由。

Rewrite `web/src/components/StatusBar.tsx` 全文：

```tsx
import { selectPendingApprovalCount, useSessionManager } from "../state/sessionManager";

/** 底部状态栏：daemon 连接 · 运行状态 · 待审批数 · 模型。 */
export function StatusBar() {
  const connection = useSessionManager((s) => s.connection);
  const modelName = useSessionManager((s) => s.modelName);
  const pendingApprovals = useSessionManager(selectPendingApprovalCount);
  const activeStatus = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.status : undefined,
  );
  const isRunning = activeStatus === "running" || activeStatus === "awaiting_approval";

  const statusText =
    connection === "connected" ? "online" : connection === "disconnected" ? "offline" : "connecting";

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-border bg-background px-3 text-[11px] text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span
          className={
            connection === "connected"
              ? "h-1.5 w-1.5 rounded-full bg-success"
              : connection === "disconnected"
                ? "h-1.5 w-1.5 rounded-full bg-danger"
                : "h-1.5 w-1.5 rounded-full bg-warning"
          }
        />
        {statusText}
      </span>
      {isRunning && <span className="text-warning">working</span>}
      {pendingApprovals > 0 && (
        <span className="rounded-sm bg-warning/20 px-1 text-warning">{pendingApprovals} approval</span>
      )}
      <div className="flex-1" />
      {modelName && <span title="active model">{modelName}</span>}
    </footer>
  );
}
```

- [ ] **Step 6: App.tsx 布局重排**

Modify `web/src/App.tsx`：return 部分（118-163 行）替换为下列结构。`SessionStoreContext.Provider` 现在只包中部聊天区；`StatusBar` 移出 Provider 放底部；`LeftRail` 暂时保留原样（Task 4 替换）；右栏占位留到 Task 6。

```tsx
  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <AppTopbar />
      <div className="flex min-h-0 flex-1">
        <LeftRail client={client} />
        <SessionStoreContext.Provider value={activeStore}>
          <div className="flex min-w-0 flex-1 flex-col">
            <SessionHeader />
            <main className="main">
              <ChatView />
            </main>
            <Composer
              onSend={(text) => {
                if (activeId) void runSessionTurn(client, activeId, text);
              }}
              onStop={() => {
                if (activeId) void stopSessionTurn(client, activeId);
              }}
              onCommand={setOpenCommand}
            />
          </div>
        </SessionStoreContext.Provider>
      </div>
      <StatusBar />
      <PermissionModal client={client} />
      <QuestionModal client={client} />
      {openCommand?.name === "/model" && (
        <CommandModal title="Switch model" onClose={closeCommand}>
          <ModelPanel client={client} />
        </CommandModal>
      )}
      {openCommand?.name === "/sessions" && (
        <SessionsBrowserModal client={client} onClose={closeCommand} />
      )}
      {openCommand?.name === "/memory" && (
        <CommandModal title="Memory" onClose={closeCommand}>
          <MemoryPanel client={client} />
        </CommandModal>
      )}
      {openCommand?.name === "/undo" && (
        <CommandModal title="Undo turn" onClose={closeCommand}>
          <CheckpointsPanel client={client} />
        </CommandModal>
      )}
      <Toaster theme="dark" position="bottom-right" />
    </div>
  );
```

同步修改 import：加 `import { AppTopbar } from "./components/layout/AppTopbar";`。

注意：`.app` / `.app-body` 旧 class 不再使用，但 `styles.css` 里它们还在（无害，Task 7 删除）。`<main className="main">` 保留——`ChatView` 的滚动容器样式仍在旧 CSS 中。

- [ ] **Step 7: 全量验证 + 手测 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿（`panels.test.tsx` 等旧测试不依赖被改的 class 名；若有断言旧 class 的测试失败，更新断言为新结构）

手测：`pnpm --dir web dev`——顶栏出现、主题切换即时生效且刷新后保持、底部状态栏显示 online/model。

```bash
git add web/src/components/ui/ web/src/components/layout/ web/src/components/StatusBar.tsx web/src/App.tsx
git commit -m "feat(web): add app shell with topbar, bottom status bar, theme switcher"
```

---

### Task 4: LeftSidebar — ProjectTree 迁入换壳

**Files:**
- Create: `web/src/components/layout/LeftSidebar.tsx`
- Rename: `web/src/components/ProjectTree.tsx` → `web/src/features/sessions/ProjectTree.tsx`（`git mv`，test 文件一并移动）
- Rename: `web/src/components/NewSessionModal.tsx` → `web/src/features/sessions/NewSessionModal.tsx`（同上）
- Rename: `web/src/components/RailSection.tsx` → `web/src/features/sessions/RailSection.tsx`（同上；SkillPanel 仍在用它，Task 6 才处理 SkillPanel）
- Modify: `web/src/App.tsx`（LeftRail → LeftSidebar）
- Delete: `web/src/components/LeftRail.tsx`

**Interfaces:**
- Consumes: `useUiStore.leftCollapsed / toggleLeft`（Task 2）
- Produces: `<LeftSidebar client: DaemonClient />`——App 消费；`<ProjectTree client>`、`<NewSessionModal>` props 不变（纯移动 + 样式改 class）。

- [ ] **Step 1: git mv 移动会话相关组件**

```bash
mkdir -p web/src/features/sessions
git mv web/src/components/ProjectTree.tsx web/src/features/sessions/ProjectTree.tsx
git mv web/src/components/ProjectTree.test.tsx web/src/features/sessions/ProjectTree.test.tsx
git mv web/src/components/NewSessionModal.tsx web/src/features/sessions/NewSessionModal.tsx
git mv web/src/components/NewSessionModal.test.tsx web/src/features/sessions/NewSessionModal.test.tsx
git mv web/src/components/RailSection.tsx web/src/features/sessions/RailSection.tsx
git mv web/src/components/RailSection.test.tsx web/src/features/sessions/RailSection.test.tsx
```

修正这些文件内部的相对 import（`../api/client` → `../../api/client`、`./RailSection` 等），以及引用它们的文件：`web/src/components/SkillPanel.tsx`（`./RailSection` → `../features/sessions/RailSection`）、`web/src/components/LeftRail.tsx`（暂时改成新路径，Step 3 删除它）。

- [ ] **Step 2: ProjectTree 样式换 Tailwind**

`ProjectTree.tsx`（282 行）不改逻辑，只把 className 从旧 CSS class 换成 Tailwind token 类。映射规则（全文适用，Task 7 复用同一表）：

| 旧 class 用途 | 新 Tailwind 类 |
|---|---|
| 面板/卡片背景 `var(--bg-elev)` | `bg-card` |
| 悬浮 `var(--bg-hover)` | `hover:bg-accent` |
| 主文字 | `text-foreground` |
| 次要文字 `var(--fg-dim)` | `text-muted-foreground` |
| 边框 | `border-border` |
| 强调色文字/按钮 | `text-primary` / `bg-primary text-primary-foreground` |
| 圆角 | `rounded-md` / `rounded-sm` |
| 状态色 ok/warn/bad | `text-success` / `text-warning` / `text-danger` |

树节点行示例（`TreeNode` 的 head 行）：

```tsx
<div className="flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
  <button type="button" aria-expanded={!collapsed} onClick={() => setCollapsed((c) => !c)}
    className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]">
    {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
    {icon}
    <span className="truncate">{title}</span>
    {subtitle && <span className="truncate text-muted-foreground">{subtitle}</span>}
    {count !== undefined && <span className="ml-auto text-[11px] text-muted-foreground">{count}</span>}
  </button>
  {actions && <span className="flex items-center gap-0.5">{actions}</span>}
</div>
```

验证：`pnpm --dir web vitest run src/features/sessions/`——ProjectTree/NewSessionModal/RailSection 的现有测试全绿（它们断言文本与 role，不断言旧 class；如有 class 断言则更新）。

- [ ] **Step 3: 实现 LeftSidebar 并接入 App**

Create `web/src/components/layout/LeftSidebar.tsx`:

```tsx
import { PanelLeftOpen } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { ProjectTree } from "../../features/sessions/ProjectTree";
import { useUiStore } from "../../state/uiStore";
import { Button } from "../ui/button";

/** 左侧栏：会话树（project → worktree → session）。折叠状态在 uiStore；
 *  移动端抽屉行为在 Task 7 统一处理。 */
export function LeftSidebar({ client }: { client: DaemonClient }) {
  const collapsed = useUiStore((s) => s.leftCollapsed);
  const toggleLeft = useUiStore((s) => s.toggleLeft);

  if (collapsed) {
    return (
      <aside className="flex w-8 shrink-0 flex-col items-center border-r border-border bg-sidebar py-1">
        <Button variant="ghost" size="icon" onClick={toggleLeft} title="Show sidebar">
          <PanelLeftOpen size={15} />
        </Button>
      </aside>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground">
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        <ProjectTree client={client} />
      </div>
    </aside>
  );
}
```

Modify `web/src/App.tsx`：`import { LeftRail } ...` 改为 `import { LeftSidebar } from "./components/layout/LeftSidebar";`，JSX 中 `<LeftRail client={client} />` 改为 `<LeftSidebar client={client} />`。注意：SkillPanel 暂时从左栏消失（Task 6 移到右栏），这是预期行为。

Delete `web/src/components/LeftRail.tsx`。

- [ ] **Step 4: 全量验证 + 手测 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿

手测：左栏折叠/展开（顶栏按钮和 collapsed 态按钮都有效）、会话树交互（新建/归档/删除）不变。

```bash
git add -A web/src
git commit -m "feat(web): replace LeftRail with token-styled LeftSidebar, move session components to features/sessions"
```

---

### Task 5: SessionTabBar — 会话即 tab

**Files:**
- Modify: `web/src/state/uiStore.ts`（追加 tab 状态与 actions）
- Modify: `web/src/state/uiStore.test.ts`（追加 tab 测试）
- Create: `web/src/state/uiSync.ts`
- Create: `web/src/state/uiSync.test.ts`
- Create: `web/src/components/layout/SessionTabBar.tsx`
- Create: `web/src/components/layout/SessionTabBar.test.tsx`
- Modify: `web/src/App.tsx`（挂 TabBar、启动 uiSync、移除 SessionHeader）

**Interfaces:**
- Consumes: `useSessionManager`（`entries`、`activeId`、`setActive`）
- Produces: uiStore 追加 `openTabs: string[]`、`openTab(id)`、`closeTab(id): string | null`、`moveTab(id, targetId)`、`pruneTabs(ids)`
- Produces: `startUiSync(): () => void`（`state/uiSync.ts`）——App 的 effect 调用一次
- Produces: `<SessionTabBar />`（无 props）
- **移除** `<SessionHeader />`（tab 已承载名称 + 状态点；文件保留到 Task 7 清理）

- [ ] **Step 1: 写 tab actions 的失败测试（追加到 uiStore.test.ts）**

```ts
describe("uiStore tabs", () => {
  beforeEach(() => {
    useUiStore.setState({ openTabs: [] });
  });

  it("openTab appends once", () => {
    useUiStore.getState().openTab("a");
    useUiStore.getState().openTab("a");
    useUiStore.getState().openTab("b");
    expect(useUiStore.getState().openTabs).toEqual(["a", "b"]);
  });

  it("closeTab returns the neighbor to activate", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    expect(useUiStore.getState().closeTab("b")).toBe("c");
    expect(useUiStore.getState().openTabs).toEqual(["a", "c"]);
    expect(useUiStore.getState().closeTab("c")).toBe("a");
    expect(useUiStore.getState().closeTab("a")).toBeNull();
  });

  it("moveTab reorders to the target's position", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    useUiStore.getState().moveTab("a", "c");
    expect(useUiStore.getState().openTabs).toEqual(["b", "c", "a"]);
  });

  it("pruneTabs removes gone sessions", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    useUiStore.getState().pruneTabs(["b"]);
    expect(useUiStore.getState().openTabs).toEqual(["a", "c"]);
  });
});
```

Run: `pnpm --dir web vitest run src/state/uiStore.test.ts`
Expected: FAIL（actions 不存在）

- [ ] **Step 2: 实现 tab actions（追加到 uiStore.ts）**

`UiState` 接口追加：

```ts
  openTabs: string[];
  openTab: (id: string) => void;
  /** 移除 tab；返回应激活的邻居 id（无剩余 tab 时为 null）。 */
  closeTab: (id: string) => string | null;
  /** 把 id 拖到 targetId 的位置。 */
  moveTab: (id: string, targetId: string) => void;
  pruneTabs: (ids: string[]) => void;
```

store 实现追加：

```ts
  openTabs: [],
  openTab: (id) =>
    set((s) => (s.openTabs.includes(id) ? s : { openTabs: [...s.openTabs, id] })),
  closeTab: (id) => {
    let next: string | null = null;
    set((s) => {
      const idx = s.openTabs.indexOf(id);
      const openTabs = s.openTabs.filter((t) => t !== id);
      // 优先右侧邻居，删尾 tab 时取新的末尾。idx < 0 时 id 本就不在，无需激活切换。
      next = idx < 0 ? null : (openTabs[Math.min(idx, openTabs.length - 1)] ?? null);
      return { openTabs };
    });
    return next;
  },
  moveTab: (id, targetId) =>
    set((s) => {
      const from = s.openTabs.indexOf(id);
      const to = s.openTabs.indexOf(targetId);
      if (from < 0 || to < 0 || from === to) return s;
      const openTabs = s.openTabs.filter((t) => t !== id);
      openTabs.splice(to, 0, id);
      return { openTabs };
    }),
  pruneTabs: (ids) =>
    set((s) => ({ openTabs: s.openTabs.filter((t) => !ids.includes(t)) })),
```

Run: `pnpm --dir web vitest run src/state/uiStore.test.ts`
Expected: PASS

- [ ] **Step 3: 写 uiSync 的失败测试**

Create `web/src/state/uiSync.test.ts`:

```ts
import { afterEach, describe, expect, it } from "vitest";
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";
import { startUiSync } from "./uiSync";

describe("uiSync", () => {
  afterEach(() => {
    useSessionManager.setState({ entries: {}, order: [], activeId: null });
    useUiStore.setState({ openTabs: [] });
  });

  it("auto-opens a tab for the newly activated session", () => {
    const stop = startUiSync();
    const id = useSessionManager.getState().createLocalSession("S1");
    expect(useUiStore.getState().openTabs).toEqual([id]);
    stop();
  });

  it("prunes tabs of removed sessions", () => {
    const stop = startUiSync();
    const id = useSessionManager.getState().createLocalSession("S1");
    useSessionManager.getState().removeSession(id);
    expect(useUiStore.getState().openTabs).toEqual([]);
    stop();
  });
});
```

Run: `pnpm --dir web vitest run src/state/uiSync.test.ts`
Expected: FAIL

- [ ] **Step 4: 实现 uiSync.ts**

Create `web/src/state/uiSync.ts`:

```ts
/**
 * 单向同步：sessionManager → uiStore.openTabs。
 * - 会话被激活（新建/点选）时自动补开 tab
 * - 会话被删除/归档移除时剪掉对应 tab
 * 返回取消订阅函数。App 启动时调用一次。
 */
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";

export function startUiSync(): () => void {
  return useSessionManager.subscribe((s, prev) => {
    const ui = useUiStore.getState();
    if (s.activeId && s.activeId !== prev.activeId && !ui.openTabs.includes(s.activeId)) {
      ui.openTab(s.activeId);
    }
    const stale = ui.openTabs.filter((id) => !s.entries[id]);
    if (stale.length > 0) ui.pruneTabs(stale);
  });
}
```

Run: `pnpm --dir web vitest run src/state/uiSync.test.ts`
Expected: PASS

- [ ] **Step 5: 写 SessionTabBar 的失败测试**

Create `web/src/components/layout/SessionTabBar.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { SessionTabBar } from "./SessionTabBar";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";

function seed() {
  const mgr = useSessionManager.getState();
  const a = mgr.createLocalSession("Alpha");
  const b = mgr.createLocalSession("Beta");
  useSessionManager.getState().setActive(a);
  useUiStore.setState({ openTabs: [a, b] });
  return { a, b };
}

describe("SessionTabBar", () => {
  beforeEach(() => {
    useSessionManager.setState({ entries: {}, order: [], activeId: null });
    useUiStore.setState({ openTabs: [] });
  });

  it("renders a tab per open session, marks the active one", () => {
    seed();
    render(<SessionTabBar />);
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.getByText("Alpha").closest("[data-active]")).toHaveAttribute("data-active", "true");
  });

  it("clicking a tab activates its session", async () => {
    const { b } = seed();
    render(<SessionTabBar />);
    await userEvent.click(screen.getByText("Beta"));
    expect(useSessionManager.getState().activeId).toBe(b);
  });

  it("closing the active tab activates its neighbor", async () => {
    const { a, b } = seed();
    render(<SessionTabBar />);
    const tab = screen.getByText("Alpha").closest("[data-active]")!;
    await userEvent.click(tab.querySelector("[data-close]") as HTMLElement);
    expect(useUiStore.getState().openTabs).toEqual([b]);
    expect(useSessionManager.getState().activeId).toBe(b);
    expect(a).not.toBe(useSessionManager.getState().activeId);
  });
});
```

Run: `pnpm --dir web vitest run src/components/layout/SessionTabBar.test.tsx`
Expected: FAIL

- [ ] **Step 6: 实现 SessionTabBar**

Create `web/src/components/layout/SessionTabBar.tsx`:

```tsx
import { useRef } from "react";
import { X } from "lucide-react";
import { useSessionManager, type SessionStatus } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import { cn } from "../../lib/utils";

const STATUS_DOT: Record<SessionStatus, string> = {
  running: "bg-primary",
  awaiting_approval: "bg-warning",
  idle: "bg-success",
  error: "bg-danger",
};

/** 会话 tab 栏：每个打开的会话一个 tab。点击激活、中键/X 关闭、HTML5 拖拽排序。 */
export function SessionTabBar() {
  const openTabs = useUiStore((s) => s.openTabs);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const dragId = useRef<string | null>(null);

  const activate = (id: string) => useSessionManager.getState().setActive(id);

  const close = (id: string) => {
    const mgr = useSessionManager.getState();
    const next = useUiStore.getState().closeTab(id);
    if (mgr.activeId === id && next) mgr.setActive(next);
  };

  if (openTabs.length === 0) return null;

  return (
    <div className="flex h-9 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-border bg-sidebar px-1">
      {openTabs.map((id) => {
        const entry = entries[id];
        if (!entry) return null;
        const active = id === activeId;
        return (
          <div
            key={id}
            data-active={active}
            draggable
            onDragStart={() => (dragId.current = id)}
            onDragOver={(e) => e.preventDefault()}
            onDrop={() => {
              if (dragId.current && dragId.current !== id) {
                useUiStore.getState().moveTab(dragId.current, id);
              }
              dragId.current = null;
            }}
            className={cn(
              "group flex h-8 max-w-40 cursor-pointer items-center gap-1.5 rounded-t-md border border-b-0 border-transparent px-2.5 text-[12px]",
              active
                ? "border-border bg-background text-foreground"
                : "text-muted-foreground hover:bg-accent",
            )}
            onClick={() => activate(id)}
            onAuxClick={(e) => {
              if (e.button === 1) close(id);
            }}
          >
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", STATUS_DOT[entry.status])} />
            <span className="truncate">{entry.name}</span>
            <button
              type="button"
              data-close
              aria-label={`Close ${entry.name}`}
              className="ml-0.5 hidden rounded-sm p-0.5 hover:bg-accent group-hover:block"
              onClick={(e) => {
                e.stopPropagation();
                close(id);
              }}
            >
              <X size={11} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
```

Run: `pnpm --dir web vitest run src/components/layout/SessionTabBar.test.tsx`
Expected: PASS

- [ ] **Step 7: 接入 App**

Modify `web/src/App.tsx`：

1. import 追加：`import { SessionTabBar } from "./components/layout/SessionTabBar";`、`import { startUiSync } from "./state/uiSync";`；删掉 `import { SessionHeader } ...`。
2. 在 bootstrap effect 之后追加：

```ts
  // sessionManager → uiStore.openTabs 单向同步（激活补开 tab、删除剪 tab）。
  useEffect(() => startUiSync(), []);
```

3. JSX：中部容器内 `<SessionHeader />` 替换为 `<SessionTabBar />`；当 `openTabs` 为空时 ChatView 区域显示空态：

```tsx
          <div className="flex min-w-0 flex-1 flex-col">
            <SessionTabBar />
            <main className="main">
              <ChatView />
            </main>
            <Composer ... />
          </div>
```

（空态简化处理：`openTabs` 为空时 `activeStore` 仍存在，UI 照常——tab 栏返回 null 即可。不做独立空页面。）

- [ ] **Step 8: 全量验证 + 手测 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿

手测：点侧边栏会话开 tab、tab 间切换、关闭 tab、拖拽排序、归档会话后 tab 消失、新建会话自动开 tab。

```bash
git add -A web/src
git commit -m "feat(web): add session tab bar with session-as-tab model"
```

---

### Task 6: RightRail — activity bar + 面板迁移

**Files:**
- Create: `web/src/components/layout/RightRail.tsx`
- Create: `web/src/components/layout/RightRail.test.tsx`
- Rename: `web/src/components/SkillPanel.tsx` → `web/src/features/panels/SkillsPanel.tsx`（去 RailSection 包装）
- Rename: `web/src/components/MemoryPanel.tsx` → `web/src/features/panels/MemoryPanel.tsx`
- Rename: `web/src/components/CheckpointsPanel.tsx` → `web/src/features/panels/CheckpointsPanel.tsx`（test 一并移动）
- Create: `web/src/features/panels/SessionsPanel.tsx`（由 SessionsBrowserModal 内容改造）
- Create: `web/src/features/panels/TasksPanel.tsx`
- Create: `web/src/features/panels/TasksPanel.test.tsx`
- Modify: `web/src/App.tsx`（挂 RightRail；`/sessions` `/memory` `/undo` 命令改为开面板）
- Delete: `web/src/components/SessionsBrowserModal.tsx`（内容迁入 SessionsPanel 后）

**Interfaces:**
- Consumes: `useUiStore.rightPanel / setRightPanel`（Task 2）；`DaemonClient.getTodos()` / `listTasks()`（已有，`web/src/api/client.ts:226/230`）
- Produces: `<RightRail client: DaemonClient />`；各面板组件 props 均为 `{ client: DaemonClient }`

- [ ] **Step 1: git mv 移动面板组件**

```bash
mkdir -p web/src/features/panels
git mv web/src/components/SkillPanel.tsx web/src/features/panels/SkillsPanel.tsx
git mv web/src/components/MemoryPanel.tsx web/src/features/panels/MemoryPanel.tsx
git mv web/src/components/CheckpointsPanel.tsx web/src/features/panels/CheckpointsPanel.tsx
git mv web/src/components/CheckpointsPanel.test.tsx web/src/features/panels/CheckpointsPanel.test.tsx
```

修正相对 import。`SkillsPanel.tsx` 去掉 `RailSection` 包装（右栏面板自带标题），其内容改为：

```tsx
import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { SkillInfoDto } from "../../api/types";

/** 右栏 Skills 面板：只读技能列表（GET /api/v1/skills）。 */
export function SkillsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<SkillInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    client
      .listSkills()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  return (
    <div className="p-2">
      {error && <div className="p-2 text-danger">{error}</div>}
      <ul className="flex flex-col gap-1">
        {(items ?? []).map((s) => (
          <li key={s.name} title={s.source_path} className="rounded-sm px-2 py-1 hover:bg-accent">
            <div className="text-[13px]">{s.name}</div>
            <div className="truncate text-[11px] text-muted-foreground">{s.description}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
```

`MemoryPanel.tsx`、`CheckpointsPanel.tsx` 同样只换 class（映射表见 Task 4 Step 2），逻辑不动。引用它们的 `App.tsx` import 路径同步更新。

- [ ] **Step 2: SessionsPanel（由 SessionsBrowserModal 改造）**

Create `web/src/features/panels/SessionsPanel.tsx`：把 `SessionsBrowserModal.tsx`（140 行）的搜索框 + 会话列表 JSX 原样移入，去掉 `modal-backdrop`/`modal` 外壳和 `onClose` prop，props 改为 `{ client: DaemonClient }`；选择会话后的行为保持"加载并激活该会话"（沿用原文件里的点击处理），另外追加一行 `useUiStore.getState().setRightPanel(null)` 关闭面板。样式按 Task 4 映射表换 Tailwind。

改完后删除 `web/src/components/SessionsBrowserModal.tsx` 与 `web/src/components/SessionsBrowserModal.test.tsx`：测试文件同步迁移为 `web/src/features/panels/SessionsPanel.test.tsx`，渲染目标从 modal 换成 `<SessionsPanel client={...} />`，去掉关闭行为断言、保留搜索/列表断言。

- [ ] **Step 3: 写 TasksPanel 的失败测试**

Create `web/src/features/panels/TasksPanel.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TasksPanel } from "./TasksPanel";
import type { DaemonClient } from "../../api/client";

const fakeClient = {
  getTodos: vi.fn().mockResolvedValue({
    items: [{ content: "Write tests", status: "in_progress" }],
    has_open_items: true,
    display: "",
  }),
  listTasks: vi.fn().mockResolvedValue({
    tasks: [{ id: "t1", subject: "Ship redesign", status: "pending", priority: "high",
      description: "", created_at: "", updated_at: "", tags: [] }],
  }),
} as unknown as DaemonClient;

describe("TasksPanel", () => {
  it("renders todos and tasks", async () => {
    render(<TasksPanel client={fakeClient} />);
    expect(await screen.findByText("Write tests")).toBeInTheDocument();
    expect(await screen.findByText("Ship redesign")).toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web vitest run src/features/panels/TasksPanel.test.tsx`
Expected: FAIL

- [ ] **Step 4: 实现 TasksPanel**

Create `web/src/features/panels/TasksPanel.tsx`:

```tsx
import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { GetTodosResponse, TaskInfo } from "../../api/types";

/** 右栏 Tasks 面板：当前会话 todos（GET /todos）+ 后台任务列表（GET /tasks）。只读。 */
export function TasksPanel({ client }: { client: DaemonClient }) {
  const [todos, setTodos] = useState<GetTodosResponse | null>(null);
  const [tasks, setTasks] = useState<TaskInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([client.getTodos(), client.listTasks()])
      .then(([t, k]) => {
        setTodos(t);
        setTasks(k.tasks);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  if (error) return <div className="p-3 text-danger">{error}</div>;

  return (
    <div className="flex flex-col gap-3 p-2">
      <section>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">Todos</h3>
        <ul className="flex flex-col gap-0.5">
          {(todos?.items ?? []).map((t, i) => (
            <li key={i} className="flex items-center gap-2 rounded-sm px-2 py-1 text-[13px] hover:bg-accent">
              <span
                className={
                  t.status === "completed"
                    ? "h-1.5 w-1.5 rounded-full bg-success"
                    : t.status === "in_progress"
                      ? "h-1.5 w-1.5 rounded-full bg-primary"
                      : "h-1.5 w-1.5 rounded-full bg-muted-foreground"
                }
              />
              {t.content}
            </li>
          ))}
          {todos?.items.length === 0 && (
            <li className="px-2 py-1 text-[12px] text-muted-foreground">No todos</li>
          )}
        </ul>
      </section>
      <section>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">Tasks</h3>
        <ul className="flex flex-col gap-0.5">
          {(tasks ?? []).filter((t) => t.status !== "deleted").map((t) => (
            <li key={t.id} className="rounded-sm px-2 py-1 hover:bg-accent">
              <div className="text-[13px]">{t.subject}</div>
              <div className="text-[11px] text-muted-foreground">
                {t.status} · {t.priority}
              </div>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
```

Run: `pnpm --dir web vitest run src/features/panels/TasksPanel.test.tsx`
Expected: PASS

- [ ] **Step 5: 写 RightRail 的失败测试**

Create `web/src/components/layout/RightRail.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RightRail } from "./RightRail";
import { useUiStore } from "../../state/uiStore";
import type { DaemonClient } from "../../api/client";

const fakeClient = {
  listSkills: vi.fn().mockResolvedValue([]),
  getTodos: vi.fn().mockResolvedValue({ items: [], has_open_items: false, display: "" }),
  listTasks: vi.fn().mockResolvedValue({ tasks: [] }),
} as unknown as DaemonClient;

describe("RightRail", () => {
  beforeEach(() => {
    useUiStore.setState({ rightPanel: null });
  });

  it("renders only the activity bar when no panel is open", () => {
    render(<RightRail client={fakeClient} />);
    expect(screen.getByTitle("Skills")).toBeInTheDocument();
    expect(screen.queryByTestId("right-panel-host")).not.toBeInTheDocument();
  });

  it("opens the skills panel via its activity icon", async () => {
    render(<RightRail client={fakeClient} />);
    await userEvent.click(screen.getByTitle("Skills"));
    expect(useUiStore.getState().rightPanel).toBe("skills");
    expect(screen.getByTestId("right-panel-host")).toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web vitest run src/components/layout/RightRail.test.tsx`
Expected: FAIL

- [ ] **Step 6: 实现 RightRail**

Create `web/src/components/layout/RightRail.tsx`:

```tsx
import { Brain, History, ListTodo, Sparkles, Undo2, type LucideIcon } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { useUiStore, type RightPanelId } from "../../state/uiStore";
import { cn } from "../../lib/utils";
import { SkillsPanel } from "../../features/panels/SkillsPanel";
import { MemoryPanel } from "../../features/panels/MemoryPanel";
import { CheckpointsPanel } from "../../features/panels/CheckpointsPanel";
import { SessionsPanel } from "../../features/panels/SessionsPanel";
import { TasksPanel } from "../../features/panels/TasksPanel";

const ITEMS: { id: RightPanelId; icon: LucideIcon; label: string }[] = [
  { id: "sessions", icon: History, label: "Sessions" },
  { id: "skills", icon: Sparkles, label: "Skills" },
  { id: "memory", icon: Brain, label: "Memory" },
  { id: "checkpoints", icon: Undo2, label: "Checkpoints" },
  { id: "tasks", icon: ListTodo, label: "Tasks" },
];

const PANEL_TITLE: Record<RightPanelId, string> = {
  sessions: "Sessions",
  skills: "Skills",
  memory: "Memory",
  checkpoints: "Checkpoints",
  tasks: "Tasks",
};

/** 右栏：36px activity bar + 可切换面板。点已激活图标收起（uiStore.toggleRightPanel）。 */
export function RightRail({ client }: { client: DaemonClient }) {
  const rightPanel = useUiStore((s) => s.rightPanel);
  const toggleRightPanel = useUiStore((s) => s.toggleRightPanel);

  return (
    <div className="flex shrink-0 border-l border-border">
      {rightPanel && (
        <div data-testid="right-panel-host" className="flex w-72 flex-col bg-sidebar">
          <div className="flex h-9 shrink-0 items-center border-b border-border px-3 text-[12px] font-semibold">
            {PANEL_TITLE[rightPanel]}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {rightPanel === "sessions" && <SessionsPanel client={client} />}
            {rightPanel === "skills" && <SkillsPanel client={client} />}
            {rightPanel === "memory" && <MemoryPanel client={client} />}
            {rightPanel === "checkpoints" && <CheckpointsPanel client={client} />}
            {rightPanel === "tasks" && <TasksPanel client={client} />}
          </div>
        </div>
      )}
      <div className="flex w-9 flex-col items-center gap-0.5 bg-sidebar py-1">
        {ITEMS.map(({ id, icon: Icon, label }) => (
          <button
            key={id}
            type="button"
            title={label}
            onClick={() => toggleRightPanel(id)}
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md",
              rightPanel === id
                ? "bg-sidebar-accent text-foreground"
                : "text-muted-foreground hover:bg-sidebar-accent hover:text-foreground",
            )}
          >
            <Icon size={15} />
          </button>
        ))}
      </div>
    </div>
  );
}
```

Run: `pnpm --dir web vitest run src/components/layout/RightRail.test.tsx`
Expected: PASS

- [ ] **Step 7: App 接线 + 斜杠命令改行为**

Modify `web/src/App.tsx`：

1. import 追加 RightRail 与各面板新路径；删掉 `SessionsBrowserModal`、`MemoryPanel`、`CheckpointsPanel`（components/ 下旧路径）的 import 与 JSX。
2. `openCommand` state 只保留 `/model`；`onCommand` 处理器改为：

```ts
  const handleCommand = (cmd: SlashCommand) => {
    const ui = useUiStore.getState();
    switch (cmd.name) {
      case "/model":
        setOpenCommand(cmd);
        break;
      case "/sessions":
        ui.toggleRightPanel("sessions");
        break;
      case "/memory":
        ui.toggleRightPanel("memory");
        break;
      case "/undo":
        ui.toggleRightPanel("checkpoints");
        break;
    }
  };
```

（`useUiStore` import 追加；`Composer onCommand={handleCommand}`。）

3. JSX 中部之后、StatusBar 之前插入 `<RightRail client={client} />`（在 flex 行内、聊天区右侧）。模态框只剩 `/model` 一个 `CommandModal` + `ModelPanel`。
4. `web/src/components/slashCommands.ts` 的 description 更新：`/sessions` → "Open sessions panel"、`/memory` → "Open memory panel"、`/undo` → "Open checkpoints panel"。

- [ ] **Step 8: 全量验证 + 手测 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build`
Expected: 全绿

手测：activity bar 五个图标切换/收起；`/sessions` `/memory` `/undo` 打开对应面板；`/model` 仍开模态框。

```bash
git add -A web/src
git commit -m "feat(web): add right rail with activity bar and panel-ized slash commands"
```

---

### Task 7: 聊天区 Tailwind 化 + 删除 styles.css

**Files:**
- Modify: `web/src/components/ChatView.tsx`、`Composer.tsx`、`ToolCallCard.tsx`、`DiffView.tsx`、`CodeBlock.tsx`、`PermissionModal.tsx`、`QuestionModal.tsx`、`CommandModal.tsx`、`diffUtils.ts`（如有 class 引用）
- Modify: `web/src/features/sessions/NewSessionModal.tsx`（Task 4 未覆盖的残留 class）
- Move: `web/src/components/{ChatView,Composer,ToolCallCard,DiffView,CodeBlock,diffUtils}.tsx` → `web/src/features/chat/`；`{PermissionModal,QuestionModal}.tsx` → `web/src/features/permissions/`；`CommandModal.tsx`、`ModelPanel.tsx` → `web/src/features/panels/`（`git mv`，test 文件随动）
- Modify: `web/src/App.tsx`（import 路径）、`web/src/main.tsx`（删 `./styles.css` 引入）
- Delete: `web/src/styles.css`、`web/src/components/SessionHeader.tsx`（Task 5 后已无引用）

**Interfaces:**
- Consumes: Task 4 的 class 映射表；`cn`、ui 组件
- Produces: 全部组件 props 不变（纯样式 + 移动）；`styles.css` 删除后无任何残留引用

- [ ] **Step 1: git mv 归类**

```bash
mkdir -p web/src/features/chat web/src/features/permissions
git mv web/src/components/ChatView.tsx web/src/components/Composer.tsx web/src/components/Composer.test.tsx \
  web/src/components/ToolCallCard.tsx web/src/components/DiffView.tsx web/src/components/CodeBlock.tsx \
  web/src/components/diffUtils.ts web/src/features/chat/
git mv web/src/components/PermissionModal.tsx web/src/components/QuestionModal.tsx web/src/features/permissions/
git mv web/src/components/CommandModal.tsx web/src/components/ModelPanel.tsx web/src/features/panels/
```

修正全部受影响 import（`App.tsx`、`sessionContext` 无关；用 `pnpm --dir web typecheck` 找出所有断点逐个修）。

- [ ] **Step 2: 逐组件换 Tailwind 类**

按 Task 4 映射表逐个改。两个代表性转换：

`ChatView.tsx` 消息行（原为 `.msg .msg-user/.msg-assistant` 等 class）：

```tsx
<div className={cn("flex flex-col gap-1 px-4 py-2", role === "user" && "items-end")}>
  <div className={cn(
    "max-w-[85%] rounded-lg px-3 py-2 text-[13px]",
    role === "user" ? "bg-primary/10 text-foreground" : "bg-card text-foreground",
  )}>
```

`Composer.tsx` 输入区（原为 `.composer` 系列）：

```tsx
<div className="border-t border-border bg-background p-3">
  <div className="flex items-end gap-2 rounded-lg border border-input bg-card px-3 py-2 focus-within:ring-1 focus-within:ring-ring">
    <textarea className="max-h-40 min-h-[20px] flex-1 resize-none bg-transparent text-[13px] outline-none placeholder:text-muted-foreground" ... />
    <Button size="icon" ...>
  </div>
</div>
```

要点：
- 滚动容器：原 `.main` 的 `overflow-y: auto` → `<main className="min-h-0 flex-1 overflow-y-auto">`（同步改 `App.tsx` 中残留的 `className="main"`）
- 模态框外壳（`.modal-backdrop`/`.modal`）→ `fixed inset-0 z-50 flex items-center justify-center bg-black/50` + `w-[480px] rounded-lg border border-border bg-popover p-4`
- Markdown/reasoning/tool-card 的排版细节（代码块背景、行内 code、表格边框等）用 Tailwind 类就近表达；`CodeBlock`/`DiffView` 的 shiki 高亮配色是 JS 侧主题，不动
- streaming 光标动画：原 CSS `@keyframes` → 在 `globals.css` 追加一条（这是唯一允许新增的全局 CSS）：

```css
@keyframes pulse-cursor {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
```

配合 `animate-[pulse-cursor_1s_infinite]` 使用。

- [ ] **Step 3: 移动端抽屉**

`LeftSidebar.tsx` 补移动端行为（替代旧 `@media (max-width: 768px)` 抽屉）：用 Tailwind 响应式类——

```tsx
<aside className={cn(
  "flex w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground",
  "max-md:fixed max-md:inset-y-10 max-md:left-0 max-md:z-40 max-md:shadow-xl",
)}>
```

折叠态在移动端隐藏整个 aside（`max-md:hidden`），由顶栏 PanelLeft 按钮唤起。原 `leftrail-backdrop` 用 `max-md:fixed max-md:inset-0 max-md:z-30 max-md:bg-black/40` 的 div 实现，点击调 `toggleLeft`。

- [ ] **Step 4: 删除 styles.css 与残留**

```bash
grep -rn "styles.css" web/src   # 确认只剩 main.tsx 一处引入
```

- `web/src/main.tsx` 删除 `import "./styles.css";`
- `git rm web/src/styles.css web/src/components/SessionHeader.tsx`
- 全文搜索旧 class 名确认零残留：`grep -rn -E "className=\"[^\"]*(topbar|leftrail|session-header|modal-backdrop|command-modal|tree-node|skill-list)" web/src` 应无输出

- [ ] **Step 5: 全量验证 + 对比走查 + Commit**

Run: `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web build && pnpm --dir web lint`
Expected: 全绿（旧测试若有 class 断言失败，更新为断言新结构/文本）

对比走查清单（`pnpm --dir web dev`，与 Orca 桌面端并排）：
- [ ] 三段式布局 + 顶栏/底栏观感接近 Orca
- [ ] light/dark/system 三主题切换无样式残留（旧 CSS 变量不再生效）
- [ ] tab 开关/排序/状态点
- [ ] 右栏五面板
- [ ] 聊天流、diff、权限模态、问答模态功能回归

```bash
git add -A web/src web/index.html
git commit -m "feat(web): migrate chat surface to tailwind, drop legacy styles.css"
```

---

## Self-Review 记录

- **Spec 覆盖**：P0→Task 1，P1→Task 2/3/4，P2→Task 5，P3→Task 6，P4→Task 7。设计文档各节（布局/tab 模型/主题/迁移阶段/测试/非目标）均有对应 Task；分栏、终端等在非目标中明确排除。
- **类型一致性**：`RightPanelId`、`ThemeMode`、uiStore actions、`startUiSync`、各面板 `{ client }` props 在定义处与消费处逐一核对一致。
- **已知留白（有意为之）**：纯样式迁移步骤（Task 4 Step 2、Task 7 Step 2）给出映射表 + 代表性代码而非逐行全部代码——这些步骤的验收标准是"现有测试全绿 + 视觉走查"，逻辑零改动。
