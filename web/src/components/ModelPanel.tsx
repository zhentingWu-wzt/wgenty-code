import { useState } from "react";
import { DaemonClient } from "../api/client";
import { useSidebarStore } from "../state/sidebarStore";
import { useSessionManager } from "../state/sessionManager";

/** Model profile picker via `GET /api/v1/models` + `POST /api/v1/model/switch`. */
export function ModelPanel({ client }: { client: DaemonClient }) {
  const models = useSidebarStore((s) => s.models);
  const setModels = useSidebarStore((s) => s.setModels);
  const setModelName = useSessionManager((s) => s.setModelName);
  const [switching, setSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const res = await client.listModels();
      setModels(res.profiles);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  // Load once on first render.
  if (models.length === 0 && !error) {
    void refresh();
  }

  const onSwitch = async (key: string) => {
    setSwitching(key);
    setError(null);
    try {
      const res = await client.switchModel({ profile: key });
      setModelName(res.model_name);
      // Refresh so the `active` marker updates.
      await refresh();
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
