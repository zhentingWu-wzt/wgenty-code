import { useCallback, useEffect, useState } from "react";
import { DaemonClient } from "../../api/client";
import type { ModelOption } from "../../api/types";
import { useSessionManager } from "../../state/sessionManager";
import { cn } from "../../lib/utils";

/** Model profile picker via `GET /api/v1/models` + `POST /api/v1/model/switch`.
 *  Rendered inside the `/model` command modal. */
export function ModelPanel({ client }: { client: DaemonClient }) {
  const [models, setModels] = useState<ModelOption[]>([]);
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
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
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
    return <div className="p-2 text-[12px] text-muted-foreground">{error ?? "Loading models…"}</div>;
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
