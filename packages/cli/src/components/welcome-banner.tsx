import React from "react";
import { Box, Text } from "ink";

interface Props {
  model: string;
  width?: number;
}

const LOGO_LINES = [
  "  ▄   ▄   ▄▄▄   ▄▄▄▄▄  ▄   ▄  ▄▄▄▄▄  ▄   ▄",
  "  █   █   ███   █████  █   █  █████  █   █",
  "  █   █  █   █  █      ██  █    █    █   █",
  "  █ █ █  █      ███    █ █ █    █     ███ ",
  "  █ █ █  █  ██  █      █  ██    █      █  ",
  "   █ █    ████  █████  █   █    █      █  ",
];

const GRADIENT = [
  "rgb(220,180,255)",
  "rgb(200,160,240)",
  "rgb(170,130,220)",
  "rgb(140,100,195)",
  "rgb(115,80,170)",
  "rgb(100,60,150)",
];

export const WelcomeBanner: React.FC<Props> = ({ model, width = 80 }) => {
  const w = Math.min(width - 4, 70);
  const divider = "─".repeat(w);

  return (
    <Box flexDirection="column" marginY={1} paddingLeft={2}>
      {/* ASCII logo */}
      {LOGO_LINES.map((line, i) => (
        <Box key={i}>
          <Text bold color={GRADIENT[i]}>
            {line}
          </Text>
        </Box>
      ))}

      <Box height={1} />

      {/* Brand name */}
      <Box>
        <Text>        </Text>
        <Text bold color="rgb(200,150,255)">
          Wgenty Code
        </Text>
        <Text> · </Text>
        <Text bold color="rgb(255,140,66)">
          Rust Edition
        </Text>
      </Box>

      {/* Tagline */}
      <Box>
        <Text>           </Text>
        <Text color="rgb(147,112,219)">高性能 AI 编码助手</Text>
      </Box>

      <Box height={1} />

      {/* Model */}
      <Box>
        <Text>        </Text>
        <Text dimColor>Model: </Text>
        <Text color="rgb(220,200,255)">{model}</Text>
      </Box>

      <Box height={1} />

      {/* Feature bar */}
      <Box>
        <Text dimColor>{divider}</Text>
      </Box>
      <Box>
        <Text>
          {"     "}
          <Text color="rgb(255,200,50)">⚡</Text> 启动 <Text color="green" bold>2.5x</Text>
          {"  "}
          <Text color="rgb(100,200,255)">💾</Text> 内存 <Text color="green" bold>-60%</Text>
          {"  "}
          <Text color="rgb(255,140,66)">🚀</Text> 响应 <Text color="green" bold>+40%</Text>
        </Text>
      </Box>
      <Box>
        <Text dimColor>{divider}</Text>
      </Box>

      <Box height={1} />

    </Box>
  );
};
