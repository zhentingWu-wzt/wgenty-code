import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { SkillInfoDto } from "../../api/types";

/** 右栏 Skills 面板：技能列表（GET /api/v1/skills）+ 手动重扫（POST /skills/reload）。 */
export function SkillsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<SkillInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);

  const load = useCallback(() => {
    client
      .listSkills()
      .then((fresh) => {
        setItems(fresh);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(() => {
    load();
  }, [load]);

  const reload = useCallback(() => {
    setReloading(true);
    client
      .reloadSkills()
      .then((res) => {
        setItems(res.skills);
        setError(null);
        toast.success(`Skills reloaded (${res.count})`);
      })
      .catch((e) => {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
        toast.error(`Skills reload failed: ${msg.slice(0, 120)}`);
      })
      .finally(() => setReloading(false));
  }, [client]);

  return (
    <div className="p-2">
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="text-[11px] text-muted-foreground">
          {items ? `${items.length} skills` : "…"}
        </span>
        <button
          type="button"
          onClick={reload}
          disabled={reloading}
          title="Rescan skill directories"
          className="inline-flex items-center gap-1 rounded-sm px-2 py-1 text-[11px] hover:bg-accent disabled:opacity-50"
        >
          <RefreshCw className={`size-3 ${reloading ? "animate-spin" : ""}`} />
          {reloading ? "Reloading…" : "Reload"}
        </button>
      </div>
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
