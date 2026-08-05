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
    <section className="flex flex-col">
      <div className="flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px] text-muted-foreground"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          <span className="truncate font-medium">{title}</span>
        </button>
        {!collapsed && actions}
      </div>
      {!collapsed && children}
    </section>
  );
}
