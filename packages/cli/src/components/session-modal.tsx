import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";
import type { SessionInfo } from "@claude-code/core";

interface Props {
  sessions: SessionInfo[];
  selectedIndex: number;
  searchQuery: string;
  onSelect: (session: SessionInfo) => void;
  onDelete: (id: string) => void;
  onClose: () => void;
  onSearchChange: (query: string) => void;
  onNavigate: (delta: number) => void;
  onRename: (id: string, newName: string) => void;
}

export const SessionModal: React.FC<Props> = ({
  sessions,
  selectedIndex,
  searchQuery,
  onSelect,
  onDelete,
  onClose,
  onSearchChange,
  onNavigate,
  onRename,
}) => {
  const [renameTarget, setRenameTarget] = React.useState<string | null>(null);
  const [renameValue, setRenameValue] = React.useState("");

  const filtered = sessions.filter((s) => {
    if (searchQuery === "") return true;
    const q = searchQuery.toLowerCase();
    return (
      s.name.toLowerCase().includes(q) ||
      (s.summary ?? "").toLowerCase().includes(q)
    );
  });

  const displaySessions = filtered.slice(0, 20);
  const overflow = Math.max(0, filtered.length - 20);

  useInput((input, key) => {
    // Rename mode: capture input for the new name
    if (renameTarget !== null) {
      if (key.escape) {
        setRenameTarget(null);
        setRenameValue("");
        return;
      }
      if (key.return) {
        const trimmed = renameValue.trim();
        if (trimmed) onRename(renameTarget, trimmed);
        setRenameTarget(null);
        setRenameValue("");
        return;
      }
      if (key.backspace || key.delete) {
        setRenameValue((prev) => prev.slice(0, -1));
        return;
      }
      if (input.length === 1 && !key.ctrl && !key.meta) {
        setRenameValue((prev) => prev + input);
        return;
      }
      return;
    }

    if (key.escape) {
      onClose();
      return;
    }
    if (key.upArrow) {
      onNavigate(-1);
      return;
    }
    if (key.downArrow) {
      onNavigate(1);
      return;
    }
    if (key.return) {
      const selected = displaySessions[selectedIndex];
      if (selected) onSelect(selected);
      return;
    }
    // 'r' triggers rename on the selected session
    if (input === "r" && searchQuery === "") {
      const selected = displaySessions[selectedIndex];
      if (selected) {
        setRenameTarget(selected.id);
        setRenameValue(selected.name || "");
      }
      return;
    }
    if (key.delete || key.backspace) {
      if (searchQuery === "" && input === "") {
        const selected = displaySessions[selectedIndex];
        if (selected) onDelete(selected.id);
        return;
      }
    }
    // Append to search query
    if (input.length === 1 && !key.ctrl && !key.meta) {
      onSearchChange(searchQuery + input);
    } else if (key.backspace || key.delete) {
      onSearchChange(searchQuery.slice(0, -1));
    }
  });

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor="blue"
      paddingX={1}
      marginY={1}
    >
      {/* Header */}
      <Box>
        <Text bold color="blue">
          {"◇ Sessions (" + sessions.length + ")"}
        </Text>
        <Text dimColor>  esc to close</Text>
      </Box>

      {/* Search bar / Rename input */}
      <Box marginTop={1}>
        {renameTarget !== null ? (
          <>
            <Text dimColor>Rename: </Text>
            <Text>{renameValue}</Text>
            <Text dimColor>█</Text>
            <Text dimColor>  ↩ confirm  esc cancel</Text>
          </>
        ) : (
          <>
            <Text dimColor>/ </Text>
            <Text>{searchQuery}</Text>
            <Text dimColor>█</Text>
          </>
        )}
      </Box>

      {/* Divider */}
      <Box>
        <Text dimColor>{"─".repeat(40)}</Text>
      </Box>

      {/* Session list */}
      {displaySessions.length === 0 ? (
        <Box marginY={1}>
          <Text dimColor>
            {searchQuery
              ? "No sessions matching '" + searchQuery + "'"
              : "No sessions found"}
          </Text>
        </Box>
      ) : (
        <>
          {displaySessions.map((s, i) => {
            const isSelected = !renameTarget && i === selectedIndex;
            const date = new Date(s.updated_at).toLocaleDateString(undefined, {
              month: "2-digit",
              day: "2-digit",
            });
            const time = new Date(s.updated_at).toLocaleTimeString(undefined, {
              hour: "2-digit",
              minute: "2-digit",
            });

            return (
              <Box key={s.id} marginTop={i > 0 ? 1 : 0}>
                <Text color={isSelected ? "blue" : undefined}>
                  {isSelected ? "▸ " : "  "}
                </Text>
                <Box flexDirection="column">
                  <Text
                    bold={isSelected}
                    color={isSelected ? "blue" : undefined}
                  >
                    {s.name || "Untitled"}
                  </Text>
                  <Text dimColor>
                    {s.message_count + " messages · " + date + " " + time}
                    {s.summary ? " · " + s.summary.slice(0, 60) : ""}
                  </Text>
                </Box>
              </Box>
            );
          })}
          {overflow > 0 && (
            <Box marginTop={1}>
              <Text dimColor>... and {overflow} more</Text>
            </Box>
          )}
        </>
      )}

      {/* Footer shortcuts */}
      <Box marginTop={1}>
        <Text dimColor>
          {renameTarget !== null
            ? "↩ confirm · esc cancel"
            : "↑↓ navigate  ↩ load  r rename  ⌫ delete  esc close"}
        </Text>
      </Box>
    </Box>
  );
};
