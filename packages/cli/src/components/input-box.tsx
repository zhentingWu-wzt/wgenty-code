import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";
import TextInput from "ink-text-input";
import type { CompletionState } from "../hooks/use-agent";

interface Props {
  onSubmit: (value: string) => void;
  disabled?: boolean;
  width?: number;
  onToggleAll?: () => void;
  onToggleCurrent?: () => void;
}

export const InputBox: React.FC<Props> = ({
  onSubmit,
  disabled,
  width = 80,
  onToggleAll,
  onToggleCurrent,
}) => {
  const [value, setValue] = React.useState("");
  const [completionState, setCompletionState] = React.useState<CompletionState | null>(null);

  // Flag to filter out ctrl-key characters from TextInput onChange
  const ctrlFlag = React.useRef(false);

  // Keep latest onSubmit in ref so ink-text-input never sees a stale callback
  const onSubmitRef = React.useRef(onSubmit);
  onSubmitRef.current = onSubmit;

  // Keep value accessible in useInput closure via ref
  const valueRef = React.useRef(value);
  valueRef.current = value;

  const handleChange = (newValue: string) => {
    if (ctrlFlag.current) {
      ctrlFlag.current = false;
      return; // ignore ctrl-key character injected into input
    }

    setValue(newValue);

    // Update completion filter as user types more characters after @ or /
    setCompletionState((prev) => {
      if (!prev?.visible) return prev;
      const pos = newValue.lastIndexOf(prev.prefix);
      if (pos !== -1) {
        const after = newValue.slice(pos + 1);
        return { ...prev, partial: after, matches: [] };
      }
      return null;
    });
  };

  const handleSubmit = React.useCallback((val: string) => {
    const trimmed = val.trim();
    if (trimmed) {
      onSubmitRef.current(trimmed);
      setValue("");
    }
  }, []);

  useInput((input, key) => {
    // ── Completion key handling (highest priority when visible) ──────
    if (completionState?.visible) {
      if (key.escape) {
        setCompletionState(null);
        return;
      }
      if (key.upArrow || input === "k") {
        setCompletionState((prev) =>
          prev
            ? { ...prev, selectedIndex: Math.max(0, prev.selectedIndex - 1) }
            : null,
        );
        return;
      }
      if (key.downArrow || input === "j") {
        setCompletionState((prev) =>
          prev
            ? {
                ...prev,
                selectedIndex: Math.min(
                  prev.matches.length - 1,
                  prev.selectedIndex + 1,
                ),
              }
            : null,
        );
        return;
      }
      if (key.return && completionState.matches[completionState.selectedIndex]) {
        const selected = completionState.matches[completionState.selectedIndex];
        setValue(selected.text + " ");
        setCompletionState(null);
        return;
      }
      // Tab cycles to next item
      if (key.tab) {
        setCompletionState((prev) => {
          if (!prev || prev.matches.length === 0) return prev;
          return {
            ...prev,
            selectedIndex: (prev.selectedIndex + 1) % prev.matches.length,
          };
        });
        return;
      }
    }

    // ── Ctrl key handling ───────────────────────────────────────────
    if (key.ctrl) {
      if (input === "e" || input === "\x05") {
        ctrlFlag.current = true;
        onToggleAll?.();
      } else if (input === "o" || input === "\x0f") {
        ctrlFlag.current = true;
        onToggleCurrent?.();
      }
    }

    // ── Detect @ and / completion triggers ─────────────────────────
    if (input === "@") {
      const currentValue = valueRef.current;
      if (!currentValue || currentValue.endsWith(" ")) {
        setCompletionState({
          visible: true,
          prefix: "@",
          partial: "",
          matches: [],
          selectedIndex: 0,
        });
      }
    }
    if (input === "/" && valueRef.current === "") {
      setCompletionState({
        visible: true,
        prefix: "/",
        partial: "",
        matches: [],
        selectedIndex: 0,
      });
    }
  });

  return (
    <Box flexDirection="column" marginTop={1}>
      <Box
        borderStyle="round"
        borderColor="rgb(255,140,66)"
        paddingX={1}
        width={width - 4}
      >
        <Text color="rgb(255,140,66)" bold>
          ▸{" "}
        </Text>
        {disabled ? (
          <Text dimColor>Thinking...</Text>
        ) : (
          <TextInput
            value={value}
            onChange={handleChange}
            onSubmit={handleSubmit}
            placeholder="Ask anything..."
          />
        )}
      </Box>

      {completionState?.visible && completionState.matches.length > 0 && (
        <Box
          flexDirection="column"
          borderStyle="round"
          borderColor="rgb(203,166,247)"
          paddingX={1}
          width={width - 4}
        >
          {completionState.matches.slice(0, 8).map((m, i) => (
            <Text
              key={m.text}
              color={
                i === completionState.selectedIndex
                  ? "rgb(203,166,247)"
                  : undefined
              }
            >
              {m.text}{" "}
              <Text dimColor>{m.description}</Text>
            </Text>
          ))}
          <Text dimColor>{"↑↕ Tab Enter Esc"}</Text>
        </Box>
      )}

      <Box marginTop={0} paddingLeft={2}>
        <Text dimColor>
          Enter send {"·"} Ctrl+E toggle all {"·"} Ctrl+O toggle this{" "}
          {"·"} Ctrl+C exit
        </Text>
      </Box>
    </Box>
  );
};
