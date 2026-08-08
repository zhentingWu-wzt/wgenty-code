import { describe, expect, it } from "vitest";
import { filterSlashCommands, matchSlashCommand } from "./slashCommands";

describe("filterSlashCommands", () => {
  it("lists all commands on bare slash", () => {
    expect(filterSlashCommands("/")).toHaveLength(4);
  });

  it("filters by prefix, case-insensitive", () => {
    expect(filterSlashCommands("/se").map((c) => c.name)).toEqual(["/sessions"]);
    expect(filterSlashCommands("/M").map((c) => c.name)).toEqual(["/model", "/memory"]);
  });

  it("returns nothing for non-slash input or after a space", () => {
    expect(filterSlashCommands("hello")).toHaveLength(0);
    expect(filterSlashCommands("/model gpt")).toHaveLength(0);
  });
});

describe("matchSlashCommand", () => {
  it("matches exact commands (trimmed)", () => {
    expect(matchSlashCommand("/model")?.name).toBe("/model");
    expect(matchSlashCommand("  /undo ")?.name).toBe("/undo");
  });

  it("rejects prefixes and unknown commands", () => {
    expect(matchSlashCommand("/mod")).toBeNull();
    expect(matchSlashCommand("/explode")).toBeNull();
    expect(matchSlashCommand("hello")).toBeNull();
  });
});
