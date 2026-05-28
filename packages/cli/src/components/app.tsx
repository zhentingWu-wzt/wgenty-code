import React from "react";
import { Box, Text } from "ink";
import { ApiClient } from "@claude-code/core";
import { useAgent } from "../hooks/use-agent.ts";
import { ChatView } from "./chat-view.tsx";
import { StatusBar } from "./status-bar.tsx";
import { InputBox } from "./input-box.tsx";
import { PermissionModal } from "./permission-modal.tsx";
import { QuestionModal } from "./question-modal.tsx";
import { WelcomeBanner } from "./welcome-banner.tsx";

interface Props {
  workingDir?: string;
}

export const App: React.FC<Props> = ({ workingDir }) => {
  const [client, setClient] = React.useState<ApiClient | null>(null);
  const [model, setModel] = React.useState("unknown");

  React.useEffect(() => {
    const port = parseInt(process.env.CLAUDE_DAEMON_PORT ?? "8371", 10);
    const c = new ApiClient({ baseUrl: `http://127.0.0.1:${port}` });
    c.getConfig()
      .then((cfg) => setModel(cfg.model))
      .catch(() => {})
      .finally(() => setClient(c));
  }, []);

  if (!client) {
    return (
      <Box flexDirection="column" padding={1}>
        <Text>Starting daemon...</Text>
      </Box>
    );
  }

  return <AgentView client={client} model={model} workingDir={workingDir} />;
};

// Separate component so useAgent always receives a valid client
const AgentView: React.FC<{
  client: ApiClient;
  model: string;
  workingDir?: string;
}> = ({ client, model, workingDir }) => {
  const cols =
    (process.stdout as unknown as { columns?: number }).columns ?? 80;

  const {
    messages,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
  } = useAgent({ client });

  const [allExpanded, setAllExpanded] = React.useState(false);
  const [overrides, setOverrides] = React.useState<Map<number, boolean>>(
    new Map()
  );

  // Find the last collapsible tool result's index (for Ctrl+O)
  const lastCollapsibleIndex = React.useMemo(() => {
    let idx = 0;
    let last = -1;
    for (const msg of messages) {
      if (
        msg.role === "tool" &&
        msg.toolPhase === "result" &&
        msg.content.split("\n").length > 10
      ) {
        idx++;
        last = idx;
      }
    }
    return last;
  }, [messages]);

  const toggleAll = React.useCallback(() => {
    setAllExpanded((prev) => !prev);
    setOverrides(new Map());
  }, []);

  const toggleCurrent = React.useCallback(() => {
    if (lastCollapsibleIndex < 0) return;
    setOverrides((prev) => {
      const next = new Map(prev);
      const current = prev.get(lastCollapsibleIndex) ?? allExpanded;
      next.set(lastCollapsibleIndex, !current);
      return next;
    });
  }, [lastCollapsibleIndex, allExpanded]);

  const handleSubmit = (input: string) => {
    switch (input.toLowerCase()) {
      case "exit":
      case "quit":
      case "/exit":
        process.exit(0);
        break;
      case "clear":
        console.clear();
        break;
      case "reset":
        reset();
        break;
      case "help":
        sendMessage("help");
        break;
      default:
        sendMessage(input);
    }
  };

  const modal =
    pendingPermission != null ? (
      <PermissionModal
        reason={pendingPermission.reason}
        sessionRule={pendingPermission.sessionRule}
        onResolve={resolvePermission}
      />
    ) : pendingQuestion != null ? (
      <QuestionModal
        question={pendingQuestion.question}
        options={pendingQuestion.options}
        multiSelect={pendingQuestion.multiSelect}
        onResolve={resolveQuestion}
      />
    ) : null;

  return (
    <Box flexDirection="column">
      <Box height={1}>
        <Text bold>Claude Code Rust — {model}</Text>
        {workingDir ? <Text dimColor> — {workingDir}</Text> : null}
      </Box>

      <Box height={1}>
        <Text dimColor>{"─".repeat(cols)}</Text>
      </Box>

      {messages.length === 0 && status.type === "idle" && (
        <WelcomeBanner model={model} width={cols} />
      )}

      {/* Messages — flow naturally, terminal handles scrollback */}
      <ChatView messages={messages} width={cols} allExpanded={allExpanded} overrides={overrides} />

      {modal}

      <StatusBar status={status} />

      {!modal && (
        <InputBox
          onSubmit={handleSubmit}
          disabled={status.type !== "idle"}
          width={cols}
          onToggleAll={toggleAll}
          onToggleCurrent={toggleCurrent}
        />
      )}
    </Box>
  );
};
