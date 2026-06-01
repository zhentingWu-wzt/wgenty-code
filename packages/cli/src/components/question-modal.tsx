import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";
import TextInput from "ink-text-input";

interface Option {
  label: string;
  description: string;
}

interface Props {
  question: string;
  options: Option[];
  multiSelect: boolean;
  onResolve: (answers: string[]) => void;
}

export const QuestionModal: React.FC<Props> = ({
  question,
  options,
  multiSelect,
  onResolve,
}) => {
  const [selected, setSelected] = React.useState<Set<number>>(new Set());
  const [cursor, setCursor] = React.useState(0);
  const [customValue, setCustomValue] = React.useState("");

  // Use refs for input handler to avoid stale closures
  const cursorRef = React.useRef(cursor);
  cursorRef.current = cursor;
  const selectedRef = React.useRef(selected);
  selectedRef.current = selected;
  const customValueRef = React.useRef(customValue);
  customValueRef.current = customValue;

  const maxIdx = options.length; // "Other" is at index maxIdx

  useInput((_input, key) => {
    const cur = cursorRef.current;

    if (key.upArrow) {
      setCursor((c) => (c > 0 ? c - 1 : maxIdx));
      return;
    }
    if (key.downArrow) {
      setCursor((c) => (c < maxIdx ? c + 1 : 0));
      return;
    }
    if (key.escape) {
      onResolve([]);
      return;
    }

    // Enter to submit (TextInput.onSubmit handles it when Other is active)
    if (key.return && cur !== maxIdx) {
      if (multiSelect) {
        // Submit selected options
        if (selectedRef.current.size > 0) {
          onResolve(
            Array.from(selectedRef.current).map((i) => options[i].label)
          );
        }
      } else {
        // Single select: submit the highlighted option
        onResolve([options[cur].label]);
      }
      return;
    }

    // Space toggles selection in multi-select mode
    if (_input === " " && multiSelect && cur < maxIdx) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(cur)) next.delete(cur);
        else next.add(cur);
        return next;
      });
      return;
    }

    // Number keys for quick select (non-Other options)
    const num = parseInt(_input, 10);
    if (num >= 1 && num <= options.length) {
      const idx = num - 1;
      if (multiSelect) {
        setSelected((prev) => {
          const next = new Set(prev);
          if (next.has(idx)) next.delete(idx);
          else next.add(idx);
          return next;
        });
      } else {
        onResolve([options[idx].label]);
      }
    }
  });

  const isOtherActive = cursor === maxIdx;

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor="cyan"
      paddingX={1}
      marginY={1}
    >
      <Box>
        <Text bold color="cyan">
          Question
        </Text>
      </Box>
      <Box marginTop={1}>
        <Text>{question}</Text>
      </Box>
      {multiSelect && (
        <Box>
          <Text dimColor>
            [Space] toggle · [Enter] submit ({selected.size} selected)
          </Text>
        </Box>
      )}
      {!multiSelect && (
        <Box>
          <Text dimColor>[↑↓] navigate · [Enter] select</Text>
        </Box>
      )}

      <Box flexDirection="column" marginTop={1}>
        {options.map((opt, i) => {
          const isCursor = cursor === i;
          const isSelected = selected.has(i);
          const prefix = isCursor ? "❯" : " ";
          const marker = multiSelect
            ? isSelected
              ? "◉"
              : "○"
            : isCursor
            ? "●"
            : "○";
          return (
            <Box key={i}>
              <Text color={isCursor ? "rgb(255,200,100)" : undefined}>
                {prefix} {marker} {i + 1}. {opt.label}
              </Text>
              {opt.description ? (
                <Text dimColor> — {opt.description}</Text>
              ) : null}
            </Box>
          );
        })}

        {/* "Other" option — inline text input when highlighted */}
        <Box>
          <Text
            color={isOtherActive ? "rgb(255,200,100)" : undefined}
          >
            {isOtherActive ? "❯" : " "} ○ {options.length + 1}. Other —{" "}
          </Text>
          {isOtherActive ? (
            <TextInput
              value={customValue}
              onChange={setCustomValue}
              onSubmit={() => {
                const text = customValue.trim();
                if (text) onResolve([text]);
              }}
              placeholder="type custom answer..."
            />
          ) : (
            <Text dimColor>enter custom answer</Text>
          )}
        </Box>
      </Box>
    </Box>
  );
};
