import React from "react";
import { Box, Text } from "ink";
import type { AgentStatus } from "../hooks/use-agent.ts";

interface Props {
  status: AgentStatus;
}

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export const StatusBar: React.FC<Props> = React.memo(
  ({ status }) => {
    const [frame, setFrame] = React.useState(0);

    React.useEffect(() => {
      if (status.type === "idle") return;
      const timer = setInterval(() => setFrame((f) => f + 1), 150);
      return () => clearInterval(timer);
    }, [status.type]);

    switch (status.type) {
      case "idle":
        return (
          <Box height={1}>
            <Text dimColor>Ready</Text>
          </Box>
        );

      case "thinking":
        return (
          <Box height={1}>
            <Text color="rgb(255,200,100)">
              {SPINNER[frame % SPINNER.length]} Thinking...
            </Text>
          </Box>
        );

      case "streaming":
        return (
          <Box height={1}>
            <Text color="rgb(147,112,219)">
              {SPINNER[frame % SPINNER.length]} Generating...
            </Text>
          </Box>
        );

      case "executing":
        return (
          <Box height={1}>
            <Text color="cyan">
              {SPINNER[frame % SPINNER.length]} {status.toolName}
            </Text>
          </Box>
        );

      default:
        return null;
    }
  },
  (prev, next) => {
    // Only re-render when status type or toolName changes, not when content changes
    if (prev.status.type !== next.status.type) return false;
    if (
      prev.status.type === "executing" &&
      next.status.type === "executing" &&
      prev.status.toolName !== next.status.toolName
    )
      return false;
    return true;
  }
);
