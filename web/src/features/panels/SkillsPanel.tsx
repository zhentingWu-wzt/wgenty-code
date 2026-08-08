import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { SkillInfoDto } from "../../api/types";

/** 右栏 Skills 面板：只读技能列表（GET /api/v1/skills）。 */
export function SkillsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<SkillInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    client
      .listSkills()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  return (
    <div className="p-2">
      {error && <div className="p-2 text-danger">{error}</div>}
      <ul className="flex flex-col gap-1">
        {(items ?? []).map((s) => (
          <li key={s.name} title={s.source_path} className="rounded-sm px-2 py-1 hover:bg-accent">
            <div className="text-[13px]">{s.name}</div>
            <div className="truncate text-[11px] text-muted-foreground">{s.description}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
