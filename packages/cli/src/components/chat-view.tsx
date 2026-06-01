import React from "react";
import { Box, Text } from "ink";
import { Message } from "./message.tsx";
import type { UIMessage } from "../hooks/use-agent.ts";

interface Props {
  committedMessages: UIMessage[];
  streamingContent: string;
  width: number;
  allExpanded: boolean;
  overrides: Map<number, boolean>;
}

/** Merge consecutive tool messages with the same name + phase. */
function mergeMessages(messages: UIMessage[]): UIMessage[] {
  const out: UIMessage[] = [];
  for (const msg of messages) {
    const prev = out[out.length - 1];
    if (
      prev &&
      prev.role === "tool" &&
      msg.role === "tool" &&
      prev.toolPhase === msg.toolPhase &&
      prev.toolName === msg.toolName
    ) {
      const prevCount = prev.toolCallCount ?? 1;
      const thisCount = msg.toolCallCount ?? 1;
      prev.toolCallCount = prevCount + thisCount;
      if (prev.toolArgs && msg.toolArgs && prev.toolPhase === "call") {
        prev.toolArgs = prev.toolArgs + " · " + msg.toolArgs;
      }
      if (prev.toolPhase === "result") {
        prev.content = prev.content + "\n───\n" + msg.content;
      }
    } else {
      out.push({ ...msg, toolCallCount: msg.toolCallCount ?? 1 });
    }
  }
  return out;
}

export const ChatView: React.FC<Props> = React.memo(({
  committedMessages,
  streamingContent,
  width,
  allExpanded,
  overrides,
}) => {
  // Only recompute when committed messages change (not during streaming)
  const merged = React.useMemo(() => mergeMessages(committedMessages), [committedMessages]);

  // Assign collapsible indices (only to results exceeding 10 lines)
  const collapsibleMap = React.useMemo(() => {
    let collapsibleIdx = 1;
    const map: number[] = [];
    for (const msg of merged) {
      if (
        msg.role === "tool" &&
        msg.toolPhase === "result" &&
        msg.content.split("\n").length > 10
      ) {
        map.push(collapsibleIdx++);
      } else {
        map.push(-1);
      }
    }
    return map;
  }, [merged]);

  const isEmpty = merged.length === 0 && !streamingContent;

  return (
    <Box flexDirection="column">
      {isEmpty && (
        <Box flexDirection="column" marginY={1}>
          <Text dimColor>Type your message and press Enter to send.</Text>
          <Text dimColor>Shift+Enter for newline. Ctrl+C to quit.</Text>
        </Box>
      )}
      {merged.map((msg, i) => {
        const ci = collapsibleMap[i];
        const expanded = overrides.get(ci) ?? allExpanded;
        return (
          <Message
            key={msg.id}
            msg={msg}
            width={width}
            expanded={expanded}
            collapseIndex={ci}
          />
        );
      })}
      {streamingContent && (
        <Message
          key="streaming"
          msg={{ id: -1, role: "assistant", content: streamingContent }}
          width={width}
        />
      )}
    </Box>
  );
});
