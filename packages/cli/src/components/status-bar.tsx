import React from "react";
import { Box, Text } from "ink";
import type { AgentStatus } from "../hooks/use-agent.ts";

interface Props {
  status: AgentStatus;
}

export const StatusBar: React.FC<Props> = React.memo(
  ({ status }) => {
    switch (status.type) {
      case "idle":
        return (
          <Box height={1}>
            <Text dimColor>Ready</Text>
          </Box>
        );

      case "connecting":
        return (
          <Box height={1}>
            <Text color="rgb(255,200,100)">{status.message}</Text>
          </Box>
        );

      case "thinking":
        return (
          <Box height={1}>
            <Text color="rgb(255,200,100)">Thinking...</Text>
          </Box>
        );

      case "streaming":
        return (
          <Box height={1}>
            <Text color="rgb(147,112,219)">Generating...</Text>
          </Box>
        );

      case "retrying":
        return (
          <Box height={1}>
            <Text color="rgb(255,165,0)">
              Reconnecting (attempt {status.attempt}/{status.maxRetries})...
            </Text>
          </Box>
        );

      case "executing":
        return (
          <Box height={1}>
            <Text color="cyan">{status.toolName}</Text>
          </Box>
        );

      default:
        return null;
    }
  },
  (prev, next) => {
    if (prev.status.type !== next.status.type) return false;
    if (prev.status.type === "executing" && next.status.type === "executing") {
      return prev.status.toolName === next.status.toolName;
    }
    return true;
  }
);
