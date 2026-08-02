import { useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { SkillInfoDto } from "../api/types";
import { RailSection } from "./RailSection";

/** Read-only skill list (GET /api/v1/skills). Enable/disable is out of scope:
 *  the knowledge layer has no enabled concept. Collapsed by default. */
export function SkillPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<SkillInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    client
      .listSkills()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  return (
    <RailSection title="Skills" defaultCollapsed>
      {error && <div className="panel-error">{error}</div>}
      <ul className="skill-list">
        {(items ?? []).map((s) => (
          <li key={s.name} className="skill-item" title={s.source_path}>
            <span className="skill-name">{s.name}</span>
            <span className="skill-desc">{s.description}</span>
          </li>
        ))}
      </ul>
    </RailSection>
  );
}
