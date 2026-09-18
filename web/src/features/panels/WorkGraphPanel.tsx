import { useCallback, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type {
  WorkGraphAuditEventDto,
  WorkGraphPlanDto,
  WorkGraphSessionSnapshot,
} from "../../api/types";
import { usePolling } from "../../hooks/usePolling";

/**
 * 右栏 Work Graph 面板：渲染 daemon 持有的 Work-Graph 运行时快照
 * （GET /workgraph，2s 轮询）。三段视图：
 *  1. 图全貌 —— SVG 分层 DAG（plan 节点/边 + 当前 stage 高亮 + 锚点相位）
 *  2. 节点链 —— 线性 node chain 的状态时间线（running/verified/failed…）
 *  3. 演进历史 —— 审计尾部事件（倒序）：锚点、路由、适配、分解
 * 不引入图库依赖：≤8 节点的有界 plan 用最长路径分层自绘即可。
 */

// ── 布局常量（SVG 自然宽度，容器横向滚动兜底 3 层以上的图） ─────────────────
const NODE_W = 84;
const NODE_H = 30;
const COL_GAP = 46;
const ROW_GAP = 12;
const PAD = 6;

/** Plan 节点角色 → 配色（与 Mermaid 渲染器的语义一致：readonly 白 / mutate 黄 / spawn 绿）。 */
const ROLE_COLORS: Record<string, { fill: string; stroke: string }> = {
  GeneralPurpose: { fill: "#e8f5e9", stroke: "#2e7d32" },
  RootCause: { fill: "#ede7f6", stroke: "#6a1b9a" },
  Verification: { fill: "#e3f2fd", stroke: "#1565c0" },
  HumanReview: { fill: "#fff8e1", stroke: "#f57c00" },
};

const DEFAULT_ROLE_COLOR = { fill: "#eceff1", stroke: "#546e7a" };

function roleColor(role: string) {
  return ROLE_COLORS[role] ?? DEFAULT_ROLE_COLOR;
}

/** 最长路径分层：源点在左，边一律从左指向右（plan 校验保证无环）。 */
function assignLayers(
  nodes: { id: string }[],
  edges: { from: string; to: string }[],
): Map<string, number> {
  const layer = new Map(nodes.map((n) => [n.id, 0]));
  for (let i = 0; i < nodes.length; i += 1) {
    let changed = false;
    for (const e of edges) {
      const next = (layer.get(e.from) ?? 0) + 1;
      if (next > (layer.get(e.to) ?? 0)) {
        layer.set(e.to, next);
        changed = true;
      }
    }
    if (!changed) break;
  }
  return layer;
}

/** route（snake_case）→ plan 节点 id。锚点相位路由没有对应 plan 节点。 */
function nodeForRoute(
  plan: WorkGraphPlanDto,
  route: string,
): string | null {
  const byId: Record<string, string> = {
    implement: "implement",
    root_cause: "diagnose",
    verify_gate: "verify",
  };
  if (byId[route] && plan.nodes.some((n) => n.id === byId[route])) {
    return byId[route];
  }
  const byRole: Record<string, string> = {
    implement: "GeneralPurpose",
    root_cause: "RootCause",
    human_review: "HumanReview",
    verify_gate: "Verification",
  };
  const role = byRole[route];
  const hit = role ? plan.nodes.find((n) => n.role === role) : undefined;
  return hit?.id ?? null;
}

/** 最近一次 route_selected 事件（当前执行 stage 的权威来源）。 */
function currentRoute(events: WorkGraphAuditEventDto[]): string | null {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    if (events[i].kind === "route_selected" && events[i].route) {
      return events[i].route;
    }
  }
  return null;
}

/** 锚点相位是否已通过（全部命令 exit 0）。 */
function anchorPassed(
  events: WorkGraphAuditEventDto[],
  anchor: string,
): boolean {
  return events.some(
    (e) =>
      e.kind === "anchor_completed" &&
      e.anchor === anchor &&
      e.commands.length > 0 &&
      e.commands.every((c) => c.exit_code === 0),
  );
}

const PHASE_LABEL: Record<string, string> = {
  compile_anchor: "Compile",
  test_anchor: "Test",
  verify_gate: "Verify",
};

const STATUS_DOT: Record<string, string> = {
  running: "bg-blue-500",
  verifying: "bg-amber-500",
  verified: "bg-green-600",
  failed: "bg-red-500",
  pending: "bg-muted-foreground",
};

const ROUTE_LABEL: Record<string, string> = {
  root_cause: "diagnose",
  implement: "implement",
  compile_anchor: "compile",
  test_anchor: "test",
  verify_gate: "verify",
  human_review: "human review",
  complete: "complete",
  escalate: "escalate",
};

function PlanGraph({
  plan,
  events,
}: {
  plan: WorkGraphPlanDto;
  events: WorkGraphAuditEventDto[];
}) {
  const layers = assignLayers(plan.nodes, plan.edges);
  const activeRoute = currentRoute(events);
  const activeNode = activeRoute ? nodeForRoute(plan, activeRoute) : null;

  // 按层分组定位。
  const byLayer = new Map<number, { id: string; role: string }[]>();
  for (const n of plan.nodes) {
    const l = layers.get(n.id) ?? 0;
    if (!byLayer.has(l)) byLayer.set(l, []);
    byLayer.get(l)!.push(n);
  }
  const maxLayer = Math.max(...byLayer.keys(), 0);
  const pos = new Map<string, { x: number; y: number }>();
  for (let l = 0; l <= maxLayer; l += 1) {
    const col = byLayer.get(l) ?? [];
    col.forEach((n, row) => {
      pos.set(n.id, { x: PAD + l * (NODE_W + COL_GAP), y: PAD + row * (NODE_H + ROW_GAP) });
    });
  }
  const width = PAD * 2 + (maxLayer + 1) * NODE_W + maxLayer * COL_GAP;
  const rowCount = Math.max(
    ...Array.from(byLayer.values()).map((c) => c.length),
    1,
  );
  const height = PAD * 2 + rowCount * NODE_H + (rowCount - 1) * ROW_GAP;

  return (
    <div className="overflow-x-auto rounded-md border border-border bg-background p-1">
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label="Work-Graph plan"
      >
        <defs>
          <marker
            id="wge-arrow"
            viewBox="0 0 8 8"
            refX={7}
            refY={4}
            markerWidth={6}
            markerHeight={6}
            orient="auto-start-reverse"
          >
            <path d="M0,0 L8,4 L0,8 z" fill="#94a3b8" />
          </marker>
        </defs>
        {plan.edges.map((e) => {
          const a = pos.get(e.from);
          const b = pos.get(e.to);
          if (!a || !b) return null;
          const x1 = a.x + NODE_W;
          const y1 = a.y + NODE_H / 2;
          const x2 = b.x;
          const y2 = b.y + NODE_H / 2;
          const mid = (x1 + x2) / 2;
          return (
            <path
              key={`${e.from}->${e.to}`}
              d={`M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`}
              fill="none"
              stroke="#94a3b8"
              strokeWidth={1.2}
              markerEnd="url(#wge-arrow)"
            />
          );
        })}
        {plan.nodes.map((n) => {
          const p = pos.get(n.id)!;
          const color = roleColor(n.role);
          const active = n.id === activeNode;
          return (
            <g key={n.id}>
              {active && (
                <rect
                  x={p.x - 3}
                  y={p.y - 3}
                  width={NODE_W + 6}
                  height={NODE_H + 6}
                  rx={7}
                  fill="none"
                  stroke={color.stroke}
                  strokeWidth={1.6}
                >
                  <animate
                    attributeName="stroke-opacity"
                    values="1;0.15;1"
                    dur="1.6s"
                    repeatCount="indefinite"
                  />
                </rect>
              )}
              <rect
                x={p.x}
                y={p.y}
                width={NODE_W}
                height={NODE_H}
                rx={5}
                fill={color.fill}
                stroke={color.stroke}
                strokeWidth={active ? 1.6 : 1}
              />
              <text
                x={p.x + NODE_W / 2}
                y={p.y + 13}
                textAnchor="middle"
                fontSize={9}
                fontWeight={600}
                fill="#1f2937"
              >
                {n.id}
              </text>
              <text
                x={p.x + NODE_W / 2}
                y={p.y + 24}
                textAnchor="middle"
                fontSize={7.5}
                fill="#4b5563"
              >
                {n.role}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}

function AnchorPhases({
  plan,
  events,
}: {
  plan: WorkGraphPlanDto;
  events: WorkGraphAuditEventDto[];
}) {
  return (
    <div className="flex flex-wrap gap-1">
      {plan.phases.map((phase) => {
        const passed = anchorPassed(events, phase.replace("_anchor", "").replace("verify_gate", "verify"));
        return (
          <span
            key={phase}
            className={
              "rounded-full border px-1.5 py-0.5 text-[10px] " +
              (passed
                ? "border-green-600/40 bg-green-50 text-green-700 dark:bg-green-950 dark:text-green-300"
                : "border-border bg-sidebar text-muted-foreground")
            }
          >
            {passed ? "✓ " : ""}
            {PHASE_LABEL[phase] ?? phase}
          </span>
        );
      })}
    </div>
  );
}

function EventLine({ event }: { event: WorkGraphAuditEventDto }) {
  const failed =
    event.commands.length > 0 && event.commands.every((c) => c.exit_code !== 0);
  let text = "";
  switch (event.kind) {
    case "profile_resolved":
      text = `profile: ${event.profile ?? "?"}`;
      break;
    case "anchor_completed":
      text = `${event.anchor ?? "?"} anchor ${failed ? "✗" : "✓"}`;
      if (failed) {
        const cmd = event.commands.find((c) => c.exit_code !== 0);
        text += cmd ? ` · ${cmd.command} (exit ${cmd.exit_code})` : "";
      }
      break;
    case "route_selected":
      text = `→ ${ROUTE_LABEL[event.route ?? ""] ?? event.route}`;
      break;
    case "adapted": {
      const reasonKey = event.adapted?.reason
        ? Object.keys(event.adapted.reason)[0]
        : "adapted";
      const streak = event.adapted?.reason
        ? Object.values(event.adapted.reason)[0]?.consecutive_failures
        : undefined;
      text = `adapted rev ${event.adapted?.revision_from ?? "?"}→${event.adapted?.revision_to ?? "?"} (${reasonKey}${streak != null ? ` ×${streak}` : ""})`;
      break;
    }
    case "decomposed":
      text = `decomposed (unit of ${event.parent_node_id ?? event.node_id})`;
      break;
  }
  return (
    <li className="flex items-start gap-2 rounded-sm px-2 py-0.5 text-[11px] hover:bg-accent">
      <span className="w-12 shrink-0 text-muted-foreground">
        n{event.node_id.replace(/\D/g, "") || event.node_id}·a{event.attempt}
      </span>
      <span className={failed ? "text-red-600 dark:text-red-400" : ""}>{text}</span>
    </li>
  );
}

function SessionSection({ snapshot }: { snapshot: WorkGraphSessionSnapshot }) {
  const s = snapshot.audit_summary;
  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-1 px-1">
        <span className="rounded-sm bg-sidebar-accent px-1.5 py-0.5 font-mono text-[10px]">
          {snapshot.plan?.template_id ?? "no plan"}
        </span>
        {snapshot.plan && (
          <span className="text-[10px] text-muted-foreground">
            rev {snapshot.plan.revision}
          </span>
        )}
        {snapshot.graph_depth > 0 && (
          <span className="text-[10px] text-muted-foreground">depth {snapshot.graph_depth}</span>
        )}
        <span className="ml-auto text-[10px] text-muted-foreground">
          {s.completed_routes}✓ · {s.test_failures + s.compile_failures + s.verify_failures}✗ ·
          {" "}{s.plan_adaptations}↻ · {s.decompositions}⑂
        </span>
      </div>

      {snapshot.plan && (
        <>
          <PlanGraph plan={snapshot.plan} events={snapshot.recent_events} />
          <AnchorPhases plan={snapshot.plan} events={snapshot.recent_events} />
        </>
      )}

      <div>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">
          Node Chain
        </h3>
        <ul className="flex flex-col gap-0.5">
          {snapshot.nodes.map((n) => (
            <li
              key={n.id}
              className="flex items-center gap-2 rounded-sm px-2 py-1 text-[12px] hover:bg-accent"
            >
              <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${STATUS_DOT[n.status] ?? "bg-muted-foreground"}`} />
              <span className="font-mono text-[11px]">{n.id}</span>
              <span className="truncate" title={n.goal}>{n.goal}</span>
              <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                {n.status}
                {n.retry_count > 0 ? ` · retry ${n.retry_count}` : ""}
              </span>
            </li>
          ))}
          {snapshot.nodes.length === 0 && (
            <li className="px-2 py-1 text-[12px] text-muted-foreground">No nodes yet</li>
          )}
        </ul>
      </div>

      {snapshot.units.length > 0 && (
        <div>
          <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">
            Decomposition
          </h3>
          <ul className="flex flex-col gap-0.5">
            {snapshot.units.map((u) => (
              <li
                key={u.unit_id}
                className="flex items-center gap-2 rounded-sm px-2 py-1 text-[12px] hover:bg-accent"
              >
                <span>
                  {u.outcome ? (u.outcome.passed ? "✓" : "✗") : "…"}
                </span>
                <span className="truncate" title={u.goal}>{u.goal}</span>
                <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                  {u.outcome ? `attempts ${u.outcome.attempts_used}` : u.plan.template_id}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">
          Evolution
        </h3>
        <ul className="flex flex-col gap-0.5">
          {[...snapshot.recent_events].reverse().map((e, i) => (
            <EventLine key={`${e.node_id}-${e.kind}-${i}`} event={e} />
          ))}
          {snapshot.recent_events.length === 0 && (
            <li className="px-2 py-1 text-[12px] text-muted-foreground">No audit events</li>
          )}
        </ul>
      </div>
    </section>
  );
}

export function WorkGraphPanel({ client }: { client: DaemonClient }) {
  const [sessions, setSessions] = useState<WorkGraphSessionSnapshot[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const res = await client.getWorkGraph();
      setSessions(res.sessions);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  // 2s poll — the graph mutates on every anchor/route/adaptation; keep the
  // panel near-live without hammering the daemon.
  usePolling(refresh, true, 2000);

  if (error) return <div className="p-3 text-danger">{error}</div>;

  const visible = sessions ?? [];
  return (
    <div className="flex flex-col gap-3 p-2">
      {visible.length === 0 ? (
        <div className="px-2 py-1 text-[12px] text-muted-foreground">
          No active Work-Graph — the graph appears once a node begins
          (begin_node).
        </div>
      ) : (
        visible.map((snapshot) => (
          <SessionSection key={snapshot.session_id} snapshot={snapshot} />
        ))
      )}
      <div className="flex flex-wrap items-center gap-2 px-1 pt-1 text-[10px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <span className="h-2 w-2 rounded-sm border" style={{ background: ROLE_COLORS.GeneralPurpose.fill, borderColor: ROLE_COLORS.GeneralPurpose.stroke }} />
          implement
        </span>
        <span className="flex items-center gap-1">
          <span className="h-2 w-2 rounded-sm border" style={{ background: ROLE_COLORS.RootCause.fill, borderColor: ROLE_COLORS.RootCause.stroke }} />
          diagnose
        </span>
        <span className="flex items-center gap-1">
          <span className="h-2 w-2 rounded-sm border" style={{ background: ROLE_COLORS.Verification.fill, borderColor: ROLE_COLORS.Verification.stroke }} />
          verify
        </span>
        <span className="flex items-center gap-1">
          <span className="h-2 w-2 rounded-sm border" style={{ background: ROLE_COLORS.HumanReview.fill, borderColor: ROLE_COLORS.HumanReview.stroke }} />
          review
        </span>
      </div>
    </div>
  );
}
