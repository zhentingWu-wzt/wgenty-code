import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkGraphPanel } from "./WorkGraphPanel";
import type { DaemonClient } from "../../api/client";
import type { WorkGraphResponse } from "../../api/types";

const plan = {
  template_id: "impl+test+risk-medium+spec-1",
  nodes: [
    { id: "diagnose", role: "RootCause" },
    { id: "implement", role: "GeneralPurpose" },
    { id: "verify", role: "Verification" },
  ],
  edges: [
    { from: "diagnose", to: "implement" },
    { from: "implement", to: "verify" },
  ],
  phases: ["compile_anchor", "test_anchor", "verify_gate"],
  revision: 2,
};

const response: WorkGraphResponse = {
  sessions: [
    {
      session_id: "session-viz",
      graph_depth: 0,
      plan,
      nodes: [
        {
          id: "n1",
          goal: "implement the graph panel",
          status: "running",
          retry_count: 1,
          start_turn_id: "turn-0",
          created_at: "2026-09-18T00:00:00Z",
        },
      ],
      units: [
        {
          unit_id: "unit-0",
          goal: "child unit goal",
          plan,
          outcome: { passed: true, attempts_used: 2, summary: "ok" },
        },
      ],
      audit_summary: {
        profiles_resolved: 1,
        anchors_completed: 2,
        compile_failures: 0,
        test_failures: 1,
        verify_failures: 0,
        root_cause_routes: 1,
        implement_routes: 2,
        completed_routes: 0,
        escalated_routes: 0,
        plan_adaptations: 1,
        decompositions: 1,
      },
      recent_events: [
        {
          node_id: "n1",
          attempt: 1,
          kind: "profile_resolved",
          anchor: null,
          route: null,
          profile: "rust",
          commands: [],
          adapted: null,
          parent_node_id: null,
          timestamp: "2026-09-18T00:00:01Z",
        },
        {
          node_id: "n1",
          attempt: 1,
          kind: "anchor_completed",
          anchor: "test",
          route: null,
          profile: "rust",
          commands: [{ command: "cargo test", exit_code: 1, stderr: "" }],
          adapted: null,
          parent_node_id: null,
          timestamp: "2026-09-18T00:00:02Z",
        },
        {
          node_id: "n1",
          attempt: 1,
          kind: "adapted",
          anchor: null,
          route: null,
          profile: null,
          commands: [],
          adapted: {
            reason: { repeated_test_anchor_failure: { consecutive_failures: 2 } },
            revision_from: 1,
            revision_to: 2,
          },
          parent_node_id: null,
          timestamp: "2026-09-18T00:00:03Z",
        },
        {
          node_id: "n1",
          attempt: 2,
          kind: "route_selected",
          anchor: null,
          route: "implement",
          profile: null,
          commands: [],
          adapted: null,
          parent_node_id: null,
          timestamp: "2026-09-18T00:00:04Z",
        },
      ],
    },
  ],
};

const fakeClient = {
  getWorkGraph: vi.fn().mockResolvedValue(response),
} as unknown as DaemonClient;

describe("WorkGraphPanel", () => {
  it("renders plan signature, node chain, and evolution", async () => {
    render(<WorkGraphPanel client={fakeClient} />);

    expect(await screen.findByText("impl+test+risk-medium+spec-1")).toBeInTheDocument();
    // SVG plan nodes (id + role labels; "diagnose"/"verify" also appear in
    // the legend, so assert on multiplicity instead of uniqueness).
    expect(screen.getAllByText("diagnose").length).toBeGreaterThan(0);
    expect(screen.getAllByText("implement").length).toBeGreaterThan(0);
    expect(screen.getAllByText("verify").length).toBeGreaterThan(0);
    // Node chain with status + retry count.
    expect(screen.getByText("implement the graph panel")).toBeInTheDocument();
    expect(screen.getByText(/running · retry 1/)).toBeInTheDocument();
    // Decomposition unit with terminal outcome.
    expect(screen.getByText("child unit goal")).toBeInTheDocument();
    // Evolution shows newest route first.
    expect(screen.getByText("→ implement")).toBeInTheDocument();
    // Adaptation history carries the revision bump and trigger.
    expect(screen.getByText(/adapted rev 1→2/)).toBeInTheDocument();
    expect(screen.getByText(/repeated_test_anchor_failure ×2/)).toBeInTheDocument();
  });

  it("renders the empty state without snapshots", async () => {
    const emptyClient = {
      getWorkGraph: vi.fn().mockResolvedValue({ sessions: [] }),
    } as unknown as DaemonClient;
    render(<WorkGraphPanel client={emptyClient} />);
    expect(
      await screen.findByText(/No active Work-Graph/),
    ).toBeInTheDocument();
  });
});
