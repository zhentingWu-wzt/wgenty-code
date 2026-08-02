import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

interface RailSectionProps {
  title: string;
  /** Optional header actions (e.g. "+ New session"), hidden while collapsed. */
  actions?: ReactNode;
  defaultCollapsed?: boolean;
  children: ReactNode;
}

/** Collapsible LeftRail section (Sessions / Worktrees / Skills). */
export function RailSection({
  title,
  actions,
  defaultCollapsed = false,
  children,
}: RailSectionProps) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);

  return (
    <section className="rail-section">
      <div className="rail-section-head">
        <button
          type="button"
          className="rail-section-toggle"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          <span className="rail-section-title">{title}</span>
        </button>
        {!collapsed && actions}
      </div>
      {!collapsed && children}
    </section>
  );
}
