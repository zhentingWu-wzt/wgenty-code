import React from "react";
import { Box, Text } from "ink";
import type { ApiClient, TodoItemInfo } from "@claude-code/core";

interface Props {
  client: ApiClient;
}

const MARKER: Record<string, string> = {
  pending: "[ ]",
  in_progress: "[>]",
  completed: "[x]",
};

const MARKER_COLOR: Record<string, string> = {
  pending: "white",
  in_progress: "yellow",
  completed: "green",
};

export const TaskPanel: React.FC<Props> = ({ client }) => {
  const [items, setItems] = React.useState<TodoItemInfo[]>([]);
  const [hasOpen, setHasOpen] = React.useState(false);
  const timerRef = React.useRef<ReturnType<typeof setInterval> | null>(null);
  const prevDataRef = React.useRef("");

  React.useEffect(() => {
    let cancelled = false;

    const fetchTodos = async () => {
      try {
        const res = await client.getTodos();
        if (cancelled) return;

        // Only update state if data actually changed
        const snapshot = JSON.stringify({ items: res.items, hasOpen: res.has_open_items });
        if (snapshot !== prevDataRef.current) {
          prevDataRef.current = snapshot;
          setItems(res.items);
          setHasOpen(res.has_open_items);
        }

        // Stop polling when no open items remain — work is done
        if (!res.has_open_items && timerRef.current) {
          clearInterval(timerRef.current);
          timerRef.current = null;
        }
      } catch {
        // daemon not ready yet
      }
    };

    fetchTodos();
    timerRef.current = setInterval(fetchTodos, 1500);

    return () => {
      cancelled = true;
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [client]);

  // Hide panel when no open items — work is complete
  if (!hasOpen) return null;

  const total = items.length;

  return (
    <Box flexDirection="column" marginTop={1}>
      <Text dimColor>{"─".repeat(40)}</Text>

      {total === 0 ? (
        <Text dimColor>Planning...</Text>
      ) : (
        <>
          {items.map((item, i) => {
            const marker = MARKER[item.status] ?? "[?]";
            const color = MARKER_COLOR[item.status] ?? "white";
            const suffix =
              item.status === "in_progress" && item.active_form
                ? ` <- ${item.active_form}`
                : "";

            return (
              <Text key={i} color={color}>
                {marker} {item.content}{suffix}
              </Text>
            );
          })}
          <Text dimColor>
            {items.filter((t) => t.status === "completed").length}/{total}{" "}
            completed
          </Text>
        </>
      )}
    </Box>
  );
};
