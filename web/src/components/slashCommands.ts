/**
 * Slash command registry for the composer — mirrors the TUI's slash-driven
 * panels (`/model`, `/sessions`, `/memory`, `/undo`; src/tui/app/input.rs).
 * Each command opens a floating modal instead of a permanent panel.
 */
export interface SlashCommand {
  name: string;
  description: string;
}

export const SLASH_COMMANDS: SlashCommand[] = [
  { name: "/model", description: "Switch model profile" },
  { name: "/sessions", description: "Browse saved sessions" },
  { name: "/memory", description: "Browse memories" },
  { name: "/undo", description: "Undo a turn's file changes" },
];

/**
 * Completion candidates for the current input. Only completes while the input
 * is a bare slash prefix (no space yet — after a space it's a message).
 */
export function filterSlashCommands(input: string): SlashCommand[] {
  if (!input.startsWith("/") || input.includes(" ")) return [];
  const q = input.slice(1).toLowerCase();
  return SLASH_COMMANDS.filter((c) => c.name.slice(1).startsWith(q));
}

/** Exact command match — Enter on this input opens the modal, not a message. */
export function matchSlashCommand(input: string): SlashCommand | null {
  const t = input.trim();
  return SLASH_COMMANDS.find((c) => c.name === t) ?? null;
}
