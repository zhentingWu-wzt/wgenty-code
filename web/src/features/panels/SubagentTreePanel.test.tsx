/**
 * SubagentTreePanel 集成测试：用 daemon 真实返回的 directory payload
 * （2026-09-18 从运行中的 daemon 抓取）驱动 useSubagentDirectory 轮询 →
 * subagentDirectoryStore → 面板渲染的完整链路，验证面板能显示子代理。
 */
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";
import { useSubagentDirectory } from "../../hooks/useSubagentDirectory";
import { SubagentTreePanel } from "./SubagentTreePanel";

/** 运行中 daemon 的真实响应（root + 1 个 completed child）。 */
const REAL_DIRECTORY = {
  session_id: "740a99dc-9eaa-4f3f-9d93-d0d8087ba9bc",
  root: {
    agent_id: "86738eaa-f44c-49dd-86bf-e24c67745a3b",
    status: "Pending",
    label: "",
    summary: null,
    cumulative_tokens: 0,
    started_at: 0,
    elapsed_ms: 0,
    round: null,
    max_rounds: null,
    depth: 0,
    children: [
      {
        agent_id: "75a701a4-84ad-4dc3-9eb9-f3b8156e7b0b",
        status: "Completed",
        label: "测试subagent功能",
        summary: "连通性测试完成",
        cumulative_tokens: 17252,
        started_at: 1789696339936,
        elapsed_ms: 23729,
        round: 2,
        max_rounds: 100,
        depth: 1,
        children: [],
      },
    ],
  },
};

function stubFetch() {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/api/v1/ui/viewers"))
      return new Response(JSON.stringify({ viewer_token: "vt-test" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    if (url.includes("/api/v1/agents/directory"))
      return new Response(JSON.stringify(REAL_DIRECTORY), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    if (url.includes("/__daemon-info")) return new Response("nf", { status: 404 });
    return new Response("not found", { status: 404 });
  });
}

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

function Harness({ client }: { client: DaemonClient }) {
  useSubagentDirectory(client);
  return <SubagentTreePanel />;
}

describe("SubagentTreePanel (real daemon payload)", () => {
  beforeEach(reset);
  afterEach(() => vi.unstubAllGlobals());

  it("已绑定 daemon 的会话：轮询目录后面板显示子代理节点", async () => {
    vi.stubGlobal("fetch", stubFetch());
    const m = useSessionManager.getState();
    m.createLocalSession("subagent能正常工作吗", {
      id: "740a99dc-9eaa-4f3f-9d93-d0d8087ba9bc",
      daemonId: "740a99dc-9eaa-4f3f-9d93-d0d8087ba9bc",
    });

    render(<Harness client={new DaemonClient()} />);

    // 轮询落地后子代理标签出现（而非"暂无子代理"空态）。
    expect(await screen.findByText("测试subagent功能")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
    expect(screen.queryByText(/本会话暂无子代理/)).not.toBeInTheDocument();
  });

  it("目录为空（root 无 children）时显示空态而非加载中", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/v1/ui/viewers"))
          return new Response(JSON.stringify({ viewer_token: "vt-test" }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        if (url.includes("/api/v1/agents/directory"))
          return new Response(
            JSON.stringify({
              session_id: "s-empty",
              root: { ...REAL_DIRECTORY.root, children: [] },
            }),
            { status: 200, headers: { "content-type": "application/json" } },
          );
        if (url.includes("/__daemon-info")) return new Response("nf", { status: 404 });
        return new Response("not found", { status: 404 });
      }),
    );
    const m = useSessionManager.getState();
    m.createLocalSession("empty session", { id: "s-empty", daemonId: "s-empty" });

    render(<Harness client={new DaemonClient()} />);

    expect(await screen.findByText(/本会话暂无子代理/)).toBeInTheDocument();
    expect(screen.queryByText(/加载中/)).not.toBeInTheDocument();
  });

  it("daemon 重启后旧 viewer token 失效：404 触发重建 viewer 并恢复轮询", async () => {
    // 旧行为：viewer token 只创建一次、永不刷新，daemon 重启后所有 scoped
    // 请求永久 404，面板卡在"加载中"。新行为：404 → 重建 viewer → 重试。
    let viewerCalls = 0;
    let directoryCalls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes("/api/v1/ui/viewers")) {
          viewerCalls += 1;
          return new Response(JSON.stringify({ viewer_token: `vt-${viewerCalls}` }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        if (url.includes("/api/v1/agents/directory")) {
          directoryCalls += 1;
          const token = new Headers(init?.headers).get("x-wgenty-viewer-token");
          if (token === "vt-1") return new Response("stale viewer", { status: 404 });
          return new Response(JSON.stringify(REAL_DIRECTORY), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        if (url.includes("/__daemon-info")) return new Response("nf", { status: 404 });
        return new Response("not found", { status: 404 });
      }),
    );
    const m = useSessionManager.getState();
    m.createLocalSession("subagent能正常工作吗", {
      id: "740a99dc-9eaa-4f3f-9d93-d0d8087ba9bc",
      daemonId: "740a99dc-9eaa-4f3f-9d93-d0d8087ba9bc",
    });

    render(<Harness client={new DaemonClient()} />);

    // 第一次目录请求 404（旧 token），自动重建 viewer 重试后面板恢复。
    expect(await screen.findByText("测试subagent功能")).toBeInTheDocument();
    expect(viewerCalls).toBe(2);
    expect(directoryCalls).toBeGreaterThanOrEqual(2);
  });
});
