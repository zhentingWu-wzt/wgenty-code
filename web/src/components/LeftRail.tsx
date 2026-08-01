import { useState } from "react";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import type { DaemonClient } from "../api/client";
import { SessionList } from "./SessionList";
import { WorktreePanel } from "./WorktreePanel";
import { SkillPanel } from "./SkillPanel";
import { ModelPanel } from "./ModelPanel";

/**
 * Left column of the command center: sessions, worktrees, skills, and a
 * footer with global controls (model). Replaces the old tabbed Sidebar.
 *
 * Collapse is a local toggle — on phone breakpoints the rail doubles as the
 * slide-over drawer (see the `@media (max-width: 768px)` rules in styles.css).
 */
export function LeftRail({ client }: { client: DaemonClient }) {
  const [collapsed, setCollapsed] = useState(false);
  const toggle = () => setCollapsed((c) => !c);

  if (collapsed) {
    return (
      <aside className="leftrail leftrail-collapsed">
        <button
          type="button"
          className="leftrail-expand-btn"
          onClick={toggle}
          title="Show sidebar"
        >
          <PanelLeftOpen size={15} />
        </button>
      </aside>
    );
  }

  return (
    <>
      {/* Mobile backdrop: only visible on phone breakpoint when the drawer is
          open. Clicking it collapses the rail. Hidden on desktop via CSS. */}
      <div className="leftrail-backdrop" onClick={toggle} aria-hidden="true" />
      <aside className="leftrail">
        <div className="leftrail-header">
          <button
            type="button"
            className="leftrail-collapse-btn"
            onClick={toggle}
            title="Hide sidebar"
          >
            <PanelLeftClose size={15} />
          </button>
        </div>
        <div className="leftrail-scroll">
          <SessionList client={client} />
          <WorktreePanel client={client} />
          <SkillPanel client={client} />
        </div>
        <div className="leftrail-footer">
          <ModelPanel client={client} />
        </div>
      </aside>
    </>
  );
}
