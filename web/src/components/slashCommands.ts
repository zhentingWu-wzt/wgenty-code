/**
 * Slash command registry for the composer — mirrors the TUI's slash-driven
 * panels (`/model`, `/sessions`, `/memory`, `/undo`; src/tui/app/input.rs).
 * `/model` opens a floating modal; the rest toggle right-rail panels.
 *
 * Skills (loaded from the daemon's GET /skills) are appended dynamically:
 * they surface in the `/` menu like built-ins, but unlike panel commands a
 * picked/entered skill command is SENT as a message (the agent-side loop
 * recognizes `/skill-name`) — see `SlashCommand.send`.
 */
export interface SlashCommand {
  name: string;
  description: string;
  /** true → typing/Enter sends it as a chat message instead of onCommand. */
  send?: boolean;
}

export const SLASH_COMMANDS: SlashCommand[] = [
  { name: "/model", description: "Switch model profile" },
  { name: "/sessions", description: "Open sessions panel" },
  { name: "/memory", description: "Open memory panel" },
  { name: "/undo", description: "Open checkpoints panel" },
];

/** Dynamic skill commands (module-level cache; set once at app bootstrap). */
let skillCommands: SlashCommand[] = [];

export function setSkillCommands(skills: { name: string; description: string }[]) {
  skillCommands = skills
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((s) => ({
      name: `/${s.name}`,
      description: s.description || "skill",
      send: true,
    }));
}

/**
 * Completion candidates for the current input. Only completes while the input
 * is a bare slash prefix (no space yet — after a space it's a message).
 */
export function filterSlashCommands(input: string): SlashCommand[] {
  if (!input.startsWith("/") || input.includes(" ")) return [];
  const q = input.slice(1).toLowerCase();
  return [...SLASH_COMMANDS, ...skillCommands].filter((c) =>
    c.name.slice(1).toLowerCase().startsWith(q),
  );
}

/** Exact command match — Enter on this input opens the modal/panel, not a message. */
export function matchSlashCommand(input: string): SlashCommand | null {
  const t = input.trim();
  return SLASH_COMMANDS.find((c) => c.name === t) ?? null;
}
