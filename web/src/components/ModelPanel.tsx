import { useCallback, useEffect, useState } from "react";
import { DaemonClient } from "../api/client";
import type { ModelOption } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

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
    return <div className="panel-empty">{error ?? "Loading models…"}</div>;
  }

  return (
    <div className="model-panel">
      {error && <div className="panel-error">{error}</div>}
      <ul className="model-list">
        {models.map((m) => (
          <li key={m.key} className={`model-item ${m.active ? "model-active" : ""}`}>
            <button
              type="button"
              className="model-row"
              onClick={() => onSwitch(m.key)}
              disabled={m.active || switching !== null}
            >
              <span className="model-label">
                {m.label}
                {m.active && <span className="model-active-tag">active</span>}
              </span>
              <span className="model-sub">
                {m.model_name}
                {m.tier ? ` · ${m.tier}` : ""}
              </span>
              <span className="model-action">
                {switching === m.key ? "…" : m.active ? "✓" : "switch"}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
