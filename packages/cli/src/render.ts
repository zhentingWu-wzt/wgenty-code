// Terminal rendering utilities for the minimal CLI.

/** Print a user message header. */
export function printUserMessage(input: string): void {
  console.log(`\n  ● ${input}`);
}

/** Print a system/info message. */
export function printInfo(msg: string): void {
  console.log(`  ◦ ${msg}`);
}

/** Print an error message. */
export function printError(msg: string): void {
  console.error(`  ✗ ${msg}`);
}

/** Print a success message. */
export function printSuccess(msg: string): void {
  console.log(`  ✓ ${msg}`);
}

/** Print a tool call with key args info. */
export function printToolStart(name: string, args: Record<string, unknown>): void {
  const detail = formatToolDetail(name, args);
  const icon = toToolIcon(name);
  console.log(`\n  ${icon} ${name}${detail}`);
}

/** Print the result of a tool execution. */
export function printToolResult(success: boolean, content?: string): void {
  const status = success ? "  ✓" : "  ✗";
  if (content && content !== "Done") {
    // Truncate long results
    const preview = content.length > 120 ? content.slice(0, 120) + "..." : content;
    console.log(`  ${status} ${preview}`);
  } else {
    console.log(`  ${status}`);
  }
}

/** Map tool name to an emoji icon. */
function toToolIcon(name: string): string {
  switch (name) {
    case "file_read": return "📖";
    case "file_write": return "📝";
    case "file_edit": return "✏️";
    case "execute_command": return "⚡";
    case "search": return "🔍";
    case "list_files": return "📂";
    case "git_operations": return "🔀";
    case "task_management": return "📋";
    case "note_edit": return "📝";
    case "view": return "👁";
    default: return "🔧";
  }
}

/** Format a human-readable detail string for a tool call. */
function formatToolDetail(
  toolName: string,
  args: Record<string, unknown>
): string {
  switch (toolName) {
    case "file_read":
    case "file_edit":
    case "file_write":
      return args.path ? ` → ${args.path}` : "";
    case "execute_command":
      return args.command ? ` → \`${args.command}\`` : "";
    case "search": {
      const pattern = args.pattern;
      const path = args.path;
      if (pattern && path) return ` → \`${pattern}\` in ${path}`;
      if (pattern) return ` → \`${pattern}\``;
      return "";
    }
    case "list_files":
      return args.path ? ` → ${args.path}` : "";
    case "git_operations":
      return args.operation ? ` → ${args.operation}` : "";
    case "view": {
      const p = args.path ?? ".";
      const d = args.depth ?? 3;
      return ` → ${p} (depth: ${d})`;
    }
    default:
      return "";
  }
}

/** Print the welcome banner — matches the original Rust CLI aesthetic. */
export function printWelcome(model: string): void {
  const logoLines = [
    "  ▄   ▄   ▄▄▄   ▄▄▄▄▄  ▄   ▄  ▄▄▄▄▄  ▄   ▄",
    "  █   █   ███   █████  █   █  █████  █   █",
    "  █   █  █   █  █      ██  █    █    █   █",
    "  █ █ █  █      ███    █ █ █    █     ███ ",
    "  █ █ █  █  ██  █      █  ██    █      █  ",
    "   █ █    ████  █████  █   █    █      █  ",
  ];

  console.log();
  for (const line of logoLines) {
    console.log(line);
  }
  console.log();
  console.log("        🟣 Wgenty Code · Rust Edition");
  console.log("           高性能 AI 编码助手");
  console.log();
  console.log(`        Model: ${model}`);
  console.log();

  const cols = process.stdout.columns ?? 80;
  const width = Math.min(cols - 4, 70);
  const divider = "─".repeat(width);
  console.log(`   ${divider}`);
  console.log("     ⚡ 启动 2.5x  💾 内存 -60%  🚀 响应 +40%");
  console.log(`   ${divider}`);
  console.log();
  console.log("     输入 help 查看命令 · 输入 exit 退出");
  console.log();
}

/** Print a thinking indicator. Returns a stop function. */
export function startThinking(): () => void {
  const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
  let i = 0;
  let stopped = false;

  const timer = setInterval(() => {
    if (stopped) return;
    process.stdout.write(`\r  ${frames[i % frames.length]} thinking...`);
    i++;
  }, 120);

  return () => {
    stopped = true;
    clearInterval(timer);
    process.stdout.write("\r\x1b[K"); // clear line
  };
}
