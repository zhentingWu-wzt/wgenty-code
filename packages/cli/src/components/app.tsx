import React from "react";
import { Box, Text, useInput } from "ink";
import { ApiClient } from "@claude-code/core";
import type { SessionInfo } from "@claude-code/core";
import { useAgent } from "../hooks/use-agent.ts";
import { ChatView } from "./chat-view.tsx";
import { StatusBar } from "./status-bar.tsx";
import { InputBox } from "./input-box.tsx";
import { PermissionModal } from "./permission-modal.tsx";
import { QuestionModal } from "./question-modal.tsx";
import { SessionModal } from "./session-modal.tsx";
import { WelcomeBanner } from "./welcome-banner.tsx";
import { TaskPanel } from "./task-panel.tsx";


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
    committedMessages,
    streamingContent,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
    sessionListOpen,
    sessions,
    loadSession,
    openSessionList,
    closeSessionList,
    deleteSession,
    renameSession,
    saveRef,
  } = useAgent({ client });

  const [allExpanded, setAllExpanded] = React.useState(false);
  const [overrides, setOverrides] = React.useState<Map<number, boolean>>(
    new Map()
  );

  const [sessionSearchQuery, setSessionSearchQuery] = React.useState("");
  const [sessionSelectedIndex, setSessionSelectedIndex] = React.useState(0);

  // Startup: try to restore the most recent session
  React.useEffect(() => {
    client.listSessions().then((list) => {
      if (list.length > 0) {
        const latest = list[0];
        const updatedAt = new Date(latest.updated_at).getTime();
        const now = Date.now();
        const hoursSinceUpdate = (now - updatedAt) / (1000 * 60 * 60);
        if (hoursSinceUpdate < 24) {
          loadSession(latest.id);
        }
      }
    }).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Save session on exit (SIGINT/SIGTERM)
  React.useEffect(() => {
    const handleExit = () => {
      saveRef.current();
    };
    process.on("SIGINT", handleExit);
    process.on("SIGTERM", handleExit);
    return () => {
      process.removeListener("SIGINT", handleExit);
      process.removeListener("SIGTERM", handleExit);
    };
  }, [saveRef]);

  // Find the last collapsible tool result's index (for Ctrl+O)
  const lastCollapsibleIndex = React.useMemo(() => {
    let idx = 0;
    let last = -1;
    for (const msg of committedMessages) {
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
  }, [committedMessages]);

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

  const handleSessionSelect = React.useCallback(
    async (session: SessionInfo) => {
      closeSessionList();
      setSessionSearchQuery("");
      setSessionSelectedIndex(0);
      await loadSession(session.id);
    },
    [loadSession, closeSessionList],
  );

  const handleSessionDelete = React.useCallback(
    async (id: string) => {
      await deleteSession(id);
      setSessionSelectedIndex(0);
    },
    [deleteSession],
  );

  const handleSessionRename = React.useCallback(
    async (id: string, newName: string) => {
      await renameSession(newName);
    },
    [renameSession],
  );

  const handleSessionNavigate = React.useCallback(
    (delta: number) => {
      setSessionSelectedIndex((prev) => {
        const q = sessionSearchQuery.toLowerCase();
        const filtered = sessions.filter(
          (s) =>
            sessionSearchQuery === "" ||
            s.name.toLowerCase().includes(q) ||
            (s.summary ?? "").toLowerCase().includes(q),
        );
        const max = Math.max(0, filtered.length - 1);
        return Math.max(0, Math.min(max, prev + delta));
      });
    },
    [sessions, sessionSearchQuery],
  );

  // Ctrl+S to open session list (only when idle and no modal open)
  useInput((input, key) => {
    if (
      key.ctrl &&
      input === "s" &&
      status.type === "idle" &&
      !pendingPermission &&
      !pendingQuestion &&
      !sessionListOpen
    ) {
      openSessionList();
    }
  });

  const modal = sessionListOpen ? (
    <SessionModal
      sessions={sessions}
      selectedIndex={sessionSelectedIndex}
      searchQuery={sessionSearchQuery}
      onSelect={handleSessionSelect}
      onDelete={handleSessionDelete}
      onClose={() => {
        closeSessionList();
        setSessionSearchQuery("");
        setSessionSelectedIndex(0);
      }}
      onSearchChange={setSessionSearchQuery}
      onNavigate={handleSessionNavigate}
      onRename={handleSessionRename}
    />
  ) : pendingPermission != null ? (
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

      {committedMessages.length === 0 && !streamingContent && status.type === "idle" && (
        <WelcomeBanner model={model} width={cols} />
      )}

      <ChatView committedMessages={committedMessages} streamingContent={streamingContent} width={cols} allExpanded={allExpanded} overrides={overrides} />

      {modal}

      <TaskPanel client={client} key={committedMessages.filter(m => m.role === 'user').length} />

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
