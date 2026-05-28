import React from "react";
import { Box, Text } from "ink";
import type { UIMessage } from "../hooks/use-agent.ts";

interface Props {
  msg: UIMessage;
  width: number;
  expanded?: boolean;
  collapseIndex?: number;
}

const COLLAPSE_LINES = 10;

export const Message: React.FC<Props> = ({
  msg,
  width,
  expanded = false,
  collapseIndex = -1,
}) => {
  const maxW = Math.max(40, width - 4);

  switch (msg.role) {
    case "user":
      return (
        <Box flexDirection="column" marginY={1}>
          <Box>
            <Text color="rgb(255,140,66)" bold>
              ▸ You
            </Text>
          </Box>
          <Box
            borderStyle="single"
            borderColor="rgb(255,140,66)"
            paddingX={1}
            width={maxW}
          >
            <Text wrap="wrap">{wrapText(msg.content, maxW - 4)}</Text>
          </Box>
        </Box>
      );

    case "assistant": {
      if (!msg.content) return null;

      return (
        <Box flexDirection="column" marginY={1}>
          <Box>
            <Text color="rgb(147,112,219)" bold>
              ● Wgenty
            </Text>
          </Box>
          <Box
            borderStyle="single"
            borderColor="rgb(147,112,219)"
            paddingX={1}
            width={maxW}
          >
            <Text wrap="wrap">{msg.content}</Text>
          </Box>
        </Box>
      );
    }

    case "tool": {
      const isCall = msg.toolPhase === "call";
      const count = msg.toolCallCount ?? 1;
      const merged = count > 1;
      const borderColor = isCall
        ? "rgb(100,180,220)"
        : msg.toolSuccess
        ? "green"
        : "red";
      const icon = isCall ? "⚙" : msg.toolSuccess ? "✓" : "✗";
      const label = msg.toolName ?? "tool";
      const countLabel = merged ? ` (×${count})` : "";

      // For tool results, compute collapsed content
      const resultContent = isCall ? null : computeCollapsed(msg.content, expanded, collapseIndex);

      return (
        <Box flexDirection="column" marginY={1}>
          <Box>
            <Text color={borderColor} bold>
              {"  "}{icon} {label}{countLabel}
            </Text>
            {msg.toolArgs && isCall ? (
              <Text dimColor> — {msg.toolArgs}</Text>
            ) : null}
          </Box>
          {/* Show body for tool results */}
          {resultContent ? (
            <Box
              borderStyle="single"
              borderColor={borderColor}
              paddingX={1}
              width={maxW}
              flexDirection="column"
            >
              {resultContent.lines.map((line, i) => (
                <Text key={i} wrap="wrap" dimColor={msg.toolSuccess}>
                  {line || " "}
                </Text>
              ))}
              {resultContent.hint && (
                <Text color="yellow" dimColor>
                  {resultContent.hint}
                </Text>
              )}
            </Box>
          ) : null}
          {/* Trivial result — just content line */}
          {!isCall && !resultContent && msg.content && msg.content !== "Done" ? (
            <Box
              borderStyle="single"
              borderColor={borderColor}
              paddingX={1}
              width={maxW}
            >
              <Text wrap="wrap" dimColor={msg.toolSuccess}>
                {msg.content}
              </Text>
            </Box>
          ) : null}
        </Box>
      );
    }

    case "system":
      return (
        <Box marginY={0}>
          <Text color="yellow">{msg.content}</Text>
        </Box>
      );

    default:
      return <Text>{msg.content}</Text>;
  }
};

/** Compute display content for a tool result, handling collapse. */
function computeCollapsed(
  content: string,
  expanded: boolean,
  collapseIndex: number
): { lines: string[]; hint?: string } | null {
  if (!content) return null;

  const lines = content.split("\n");
  if (lines.length <= COLLAPSE_LINES || expanded) {
    return { lines };
  }

  // Collapsed: show first COLLAPSE_LINES lines + hint
  const shown = lines.slice(0, COLLAPSE_LINES);
  const hidden = lines.length - COLLAPSE_LINES;
  const idxTag = collapseIndex > 0 ? `[${collapseIndex}] ` : "";
  const hint = `${idxTag}${hidden} more lines — Ctrl+E all · Ctrl+O this`;
  return { lines: shown, hint };
}

function wrapText(text: string, maxWidth: number): string {
  if (maxWidth <= 0) return text;
  const lines: string[] = [];
  for (const paragraph of text.split("\n")) {
    if (paragraph.length <= maxWidth) {
      lines.push(paragraph);
    } else {
      let remaining = paragraph;
      while (remaining.length > maxWidth) {
        lines.push(remaining.slice(0, maxWidth));
        remaining = remaining.slice(maxWidth);
      }
      if (remaining.length > 0) lines.push(remaining);
    }
  }
  return lines.join("\n");
}
