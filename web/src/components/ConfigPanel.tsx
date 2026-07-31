import { useState } from "react";
import { DaemonClient } from "../api/client";

/**
 * Read-only config view from `GET /api/v1/config`. The daemon's ConfigResponse
 * already excludes secrets (it surfaces model/api_base/max_tokens/timeout/
 * streaming only — no api_key). P0 is read-only; no write controls.
 */
export function ConfigPanel({ client }: { client: DaemonClient }) {
  const [config, setConfig] = useState<import("../api/types").ConfigResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  if (!loaded) {
    setLoaded(true);
    void (async () => {
      try {
        setConfig(await client.getConfig());
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }

  if (error) return <div className="panel-error">{error}</div>;
  if (!config) return <div className="panel-empty">Loading…</div>;

  return (
    <div className="config-panel">
      <ConfigRow label="Model" value={config.model} mono />
      <ConfigRow label="API base" value={config.api_base} mono />
      <ConfigRow label="Max tokens" value={String(config.max_tokens)} />
      <ConfigRow label="Timeout" value={`${config.timeout}s`} />
      <ConfigRow label="Streaming" value={config.streaming ? "on" : "off"} />
      <div className="config-note">
        Read-only. Secrets (api_key) are never exposed by the daemon.
      </div>
    </div>
  );
}

function ConfigRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="config-row">
      <span className="config-label">{label}</span>
      <span className={`config-value ${mono ? "mono" : ""}`}>{value}</span>
    </div>
  );
}
