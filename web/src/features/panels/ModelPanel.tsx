import { useCallback, useEffect, useState } from "react";
import { DaemonClient } from "../../api/client";
import type { ModelOption } from "../../api/types";
import { useSessionManager } from "../../state/sessionManager";
import { cn } from "../../lib/utils";

/** Model profile picker via `GET /api/v1/models` + `POST /api/v1/model/switch`.
 *  Rendered inside the `/model` command modal. */
export function ModelPanel({ client }: { client: DaemonClient }) {
  const [models, setModels] = useState<ModelOption[]>([]);
  // Distinguishes "fetch in flight" from "loaded, zero profiles declared" —
  // an empty list must show the configure hint, not a forever "Loading".
  const [loading, setLoading] = useState(true);
  const setModelName = useSessionManager((s) => s.setModelName);
  const [switching, setSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listModels()
      .then((res) => {
        setModels(res.profiles);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [client]);

  useEffect(refresh, [refresh]);

  const onSwitch = async (key: string) => {
    setSwitching(key);
    setError(null);
    try {
      const res = await client.switchModel({ profile: key });
      setModelName(res.model_name);
      // Refresh so the `active` marker updates.
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSwitching(null);
    }
  };

  if (models.length === 0) {
    if (error) {
      return <div className="p-2 text-[12px] text-danger">{error}</div>;
    }
    if (loading) {
      return <div className="p-2 text-[12px] text-muted-foreground">Loading models…</div>;
    }
    // Loaded but `models.profiles` is empty — mirror the TUI picker's hint so
    // the modal explains itself instead of spinning forever.
    return (
      <div className="flex flex-col gap-2 p-2 text-[12px] text-muted-foreground">
        <div>
          No model profiles configured. Add{" "}
          <span className="font-mono text-foreground">models.profiles</span> in{" "}
          <span className="font-mono text-foreground">~/.wgenty-code/settings.json</span> to enable
          switching, e.g.:
        </div>
        <pre className="overflow-x-auto rounded-md border border-border bg-background p-2 font-mono text-[11px] text-foreground">{`"models": {
  "profiles": {
    "glm":  { "name": "glm-5.3",  "display_name": "GLM 5.3",
              "base_url": "https://…", "provider": "openai" },
    "fast": { "name": "deepseek-chat", "tier": "light" }
  }
}`}</pre>
      </div>
    );
  }

  return (
    <div>
      {error && <div className="p-2 text-danger">{error}</div>}
      <ul className="flex flex-col gap-1">
        {models.map((m) => (
          <li
            key={m.key}
            className={cn(
              "overflow-hidden rounded-md border",
              m.active ? "border-success" : "border-border",
            )}
          >
            <button
              type="button"
              className="flex w-full flex-col gap-0.5 bg-background px-2.5 py-1.5 text-left hover:enabled:bg-accent disabled:opacity-85"
              onClick={() => onSwitch(m.key)}
              disabled={m.active || switching !== null}
            >
              <span className="flex items-center gap-1.5 text-[13px] font-medium">
                {m.label}
                {m.active && (
                  <span className="rounded-sm bg-success/20 px-1 py-0.5 text-[10px] uppercase text-success">
                    active
                  </span>
                )}
              </span>
              <span className="font-mono text-[11px] text-muted-foreground">
                {m.model_name}
                {m.tier ? ` · ${m.tier}` : ""}
              </span>
              <span className="self-end text-[11px] text-primary">
                {switching === m.key ? "…" : m.active ? "✓" : "switch"}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
