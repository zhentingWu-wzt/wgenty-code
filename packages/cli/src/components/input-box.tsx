import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";
import TextInput from "ink-text-input";

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

  // Flag to filter out ctrl-key characters from TextInput onChange
  const ctrlFlag = React.useRef(false);

  // Keep latest onSubmit in ref so ink-text-input never sees a stale callback
  const onSubmitRef = React.useRef(onSubmit);
  onSubmitRef.current = onSubmit;

  useInput((input, key) => {
    if (key.ctrl) {
      if (input === "e" || input === "\x05") {
        ctrlFlag.current = true;
        onToggleAll?.();
      } else if (input === "o" || input === "\x0f") {
        ctrlFlag.current = true;
        onToggleCurrent?.();
      }
    }
  });

  const handleChange = (newValue: string) => {
    if (ctrlFlag.current) {
      ctrlFlag.current = false;
      return; // ignore ctrl-key character injected into input
    }
    setValue(newValue);
  };

  const handleSubmit = React.useCallback((val: string) => {
    const trimmed = val.trim();
    if (trimmed) {
      onSubmitRef.current(trimmed);
      setValue("");
    }
  }, []);

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

      <Box marginTop={0} paddingLeft={2}>
        <Text dimColor>
          Enter send · Ctrl+E toggle all · Ctrl+O toggle this · Ctrl+C exit
        </Text>
      </Box>
    </Box>
  );
};
