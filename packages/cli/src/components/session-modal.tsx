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
}) => {
  useInput((input, key) => {
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
      const selected = sessions[selectedIndex];
      if (selected) onSelect(selected);
      return;
    }
    if (key.delete || key.backspace) {
      if (searchQuery === "" && input === "") {
        const selected = sessions[selectedIndex];
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

  const filtered = sessions.filter(
    (s) =>
      searchQuery === "" ||
      s.name.toLowerCase().includes(searchQuery.toLowerCase()),
  );

  const displaySessions = filtered.slice(0, 20);

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

      {/* Search bar */}
      <Box marginTop={1}>
        <Text dimColor>/ </Text>
        <Text>{searchQuery}</Text>
        <Text dimColor>█</Text>
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
        displaySessions.map((s, i) => {
          const isSelected = i === selectedIndex;
          const date = new Date(s.updated_at).toLocaleDateString("zh-CN", {
            month: "2-digit",
            day: "2-digit",
          });
          const time = new Date(s.updated_at).toLocaleTimeString("zh-CN", {
            hour: "2-digit",
            minute: "2-digit",
          });

          return (
            <Box key={s.id} marginTop={i > 0 ? 1 : 0}>
              <Text color={isSelected ? "blue" : undefined}>
                {isSelected ? "▸ " : "  "}
              </Text>
              <Box flexDirection="column">
                <Text bold={isSelected} color={isSelected ? "blue" : undefined}>
                  {s.name || "Untitled"}
                </Text>
                <Text dimColor>
                  {s.message_count + " messages · " + date + " " + time}
                  {s.summary ? " · " + s.summary.slice(0, 60) : ""}
                </Text>
              </Box>
            </Box>
          );
        })
      )}

      {/* Footer shortcuts */}
      <Box marginTop={1}>
        <Text dimColor>{"↑↓ navigate  ↩ load  ⌫ delete  esc close"}</Text>
      </Box>
    </Box>
  );
};
