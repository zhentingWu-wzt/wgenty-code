/**
 * SubagentDetailPanel 重试按钮测试：失败态子代理显示"重试"按钮，点击后调用
 * POST /sessions/:id/subagents/:nodeId/retry 并展示结果提示；非失败态不显示。
 */
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { DaemonClient } from "../../api/client";
import { useSubagentDirectoryStore } from "../../state/subagentDirectoryStore";
import { SubagentDetailPanel } from "./SubagentDetailPanel";

const SESSION_ID = "sess-1";
const NODE_ID = "node-failed";

function directoryTree(childStatus: string) {
  return {
    session_id: SESSION_ID,
    root: {
      agent_id: "root-agent",
      status: "Running",
      label: "root",
      summary: null,
      cumulative_tokens: 0,
      started_at: 1,
      elapsed_ms: 1,
      round: null,
      max_rounds: null,
      depth: 0,
      children: [
        {
          agent_id: NODE_ID,
          status: childStatus,
          label: "task: analyze the auth flow",
          summary: null,
          cumulative_tokens: 500,
          started_at: 1,
          elapsed_ms: 1000,
          round: 2,
          max_rounds: 100,
          depth: 1,
          children: [],
        },
      ],
    },
  };
}

function stubFetch(
  childStatus: string,
  postCalls: { url: string }[],
  retryResponse: Record<string, unknown> = { run_id: "run-1", queued: false, resumed: true },
) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.includes("/__daemon-info")) return new Response("nf", { status: 404 });
    if (url.includes("/api/v1/ui/viewers")) {
      return new Response(JSON.stringify({ viewer_token: "vt-test" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    if (url.includes("/api/v1/agents/children/nav-1")) {
      // Navigate into the child: its own local view.
      return new Response(
        JSON.stringify({
          self_view: {
            agent_id: NODE_ID,
            status: childStatus,
            label: "task: analyze the auth flow",
            text_snapshot: null,
            cumulative_tokens: 500,
            started_at: 1,
            elapsed_ms: 1000,
            round: 2,
            max_rounds: 100,
            messages: [],
          },
          children: [],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (url.includes("/api/v1/agents/self")) {
      return new Response(
        JSON.stringify({
          self_view: {
            agent_id: "root-agent",
            status: "Running",
            label: "root",
            text_snapshot: null,
            cumulative_tokens: 0,
            started_at: 1,
            elapsed_ms: 1,
            round: null,
            max_rounds: null,
            messages: [],
          },
          children: [
            {
              agent_id: NODE_ID,
              status: childStatus,
              label: "task: analyze the auth flow",
              summary: null,
              navigation_capability: "nav-1",
              text_snapshot: null,
              cumulative_tokens: 500,
              started_at: 1,
              elapsed_ms: 1000,
              round: 2,
              max_rounds: 100,
              messages: [],
            },
          ],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (url.includes(`/subagents/${NODE_ID}/retry`) && init?.method === "POST") {
      postCalls.push({ url });
      return new Response(JSON.stringify(retryResponse), {
        status: 202,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response("not found", { status: 404 });
  });
}

function resetStores() {
  useSubagentDirectoryStore.setState({ bySession: {} });
}

function renderPanel() {
  const client = new DaemonClient();
  return render(
    <SubagentDetailPanel
      client={client}
      nodeId={NODE_ID}
      rootSessionId={SESSION_ID}
      label="task: analyze the auth flow"
    />,
  );
}

describe("SubagentDetailPanel retry button", () => {
  beforeEach(resetStores);
  afterEach(() => vi.unstubAllGlobals());

  it("失败态子代理显示重试按钮，点击后调用 retry 端点并提示已重派", async () => {
    const postCalls: { url: string }[] = [];
    vi.stubGlobal("fetch", stubFetch("Failed", postCalls));
    useSubagentDirectoryStore.setState({
      bySession: {
        [SESSION_ID]: {
          tree: directoryTree("Failed").root,
          fetchedAt: Date.now(),
          stale: false,
        },
      },
    });

    renderPanel();

    // The header retry button (distinct from the error-view refresh button).
    const button = await screen.findByRole("button", { name: /^重试$/ });
    await userEvent.click(button);

    await waitFor(() => {
      expect(
        screen.getByText(/已带上之前尝试的进度上下文/),
      ).toBeInTheDocument();
    });
    expect(postCalls).toHaveLength(1);
    expect(postCalls[0].url).toContain(
      `/api/v1/sessions/${SESSION_ID}/subagents/${NODE_ID}/retry`,
    );
  });

  it("已完成子代理不显示重试按钮", async () => {
    vi.stubGlobal("fetch", stubFetch("Completed", []));
    useSubagentDirectoryStore.setState({
      bySession: {
        [SESSION_ID]: {
          tree: directoryTree("Completed").root,
          fetchedAt: Date.now(),
          stale: false,
        },
      },
    });

    renderPanel();

    await screen.findByText("Completed");
    expect(screen.queryByRole("button", { name: /^重试$/ })).toBeNull();
  });

  it("守护进程重启后无历史上下文时，提示将从头开始", async () => {
    vi.stubGlobal(
      "fetch",
      stubFetch("Failed", [], {
        run_id: "run-2",
        queued: false,
        resumed: false,
      }),
    );
    useSubagentDirectoryStore.setState({
      bySession: {
        [SESSION_ID]: {
          tree: directoryTree("Failed").root,
          fetchedAt: Date.now(),
          stale: false,
        },
      },
    });

    renderPanel();

    const button = await screen.findByRole("button", { name: /^重试$/ });
    await userEvent.click(button);

    await waitFor(() => {
      expect(screen.getByText(/从头开始/)).toBeInTheDocument();
    });
  });
});
