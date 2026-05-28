import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";

interface Props {
  reason: string;
  sessionRule: string;
  onResolve: (choice: "allow" | "always" | "deny") => void;
}

export const PermissionModal: React.FC<Props> = ({
  reason,
  onResolve,
}) => {
  useInput((input, key) => {
    if (key.escape) {
      onResolve("deny");
      return;
    }
    switch (input.toLowerCase()) {
      case "y":
        onResolve("allow");
        break;
      case "a":
        onResolve("always");
        break;
      case "n":
        onResolve("deny");
        break;
    }
  });

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor="yellow"
      paddingX={1}
      marginY={1}
    >
      <Box>
        <Text bold color="yellow">
          Permission Required
        </Text>
      </Box>
      <Box marginTop={1}>
        <Text>{reason}</Text>
      </Box>
      <Box marginTop={1}>
        <Text dimColor>[y] Allow once [a] Always allow [n] Deny</Text>
      </Box>
    </Box>
  );
};
