import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { ConfigResponse } from "../../api/types";

/**
 * Config panel: view and edit transport-level settings (max_tokens, timeout,
 * streaming, api_base). Sensitive fields (api_key) are never shown or edited.
 *
 * PUT /config does a partial update — only changed fields are sent. The
 * response reflects the new state.
 */
export function ConfigPanel({ client }: { client: DaemonClient }) {
  const [config, setConfig] = useState<ConfigResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Local edit state — initialized from config on load, sent on save.
  const [maxTokens, setMaxTokens] = useState("");
  const [timeout, setTimeoutVal] = useState("");
  const [streaming, setStreaming] = useState(true);
  const [apiBase, setApiBase] = useState("");

  const refresh = useCallback(() => {
    client
      .getConfig()
      .then((c) => {
        setConfig(c);
        setMaxTokens(String(c.max_tokens));
        setTimeoutVal(String(c.timeout));
        setStreaming(c.streaming);
        setApiBase(c.api_base);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const hasChanges =
    config !== null &&
    (Number(maxTokens) !== config.max_tokens ||
      Number(timeout) !== config.timeout ||
      streaming !== config.streaming ||
      apiBase !== config.api_base);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const updated = await client.updateConfig({
        ...(Number(maxTokens) !== config?.max_tokens && { max_tokens: Number(maxTokens) }),
        ...(Number(timeout) !== config?.timeout && { timeout: Number(timeout) }),
        ...(streaming !== config?.streaming && { streaming }),
        ...(apiBase !== config?.api_base && { api_base: apiBase }),
      });
      setConfig(updated);
      setMaxTokens(String(updated.max_tokens));
      setTimeoutVal(String(updated.timeout));
      setStreaming(updated.streaming);
      setApiBase(updated.api_base);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  if (!config) {
    return (
      <div className="p-2">
        {error && <div className="p-2 text-danger">{error}</div>}
        {!error && <div className="p-2 text-[12px] text-muted-foreground">Loading...</div>}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-2">
      {error && <div className="p-2 text-danger">{error}</div>}

      {/* Read-only model name (switching is done via /model command, not here) */}
      <div className="flex flex-col gap-1">
        <label className="text-[11px] text-muted-foreground">Model</label>
        <input
          type="text"
          value={config.model}
          disabled
          className="rounded-sm border border-border bg-muted px-2 py-1 text-[13px] text-muted-foreground"
        />
      </div>

      {/* API base URL */}
      <div className="flex flex-col gap-1">
        <label className="text-[11px] text-muted-foreground">API Base URL</label>
        <input
          type="text"
          value={apiBase}
          onChange={(e) => setApiBase(e.target.value)}
          className="rounded-sm border border-border bg-background px-2 py-1 text-[13px] outline-none focus:border-primary"
        />
      </div>

      {/* Max tokens */}
      <div className="flex flex-col gap-1">
        <label className="text-[11px] text-muted-foreground">Max Tokens</label>
        <input
          type="number"
          min={1}
          value={maxTokens}
          onChange={(e) => setMaxTokens(e.target.value)}
          className="rounded-sm border border-border bg-background px-2 py-1 text-[13px] outline-none focus:border-primary"
        />
      </div>

      {/* Timeout */}
      <div className="flex flex-col gap-1">
        <label className="text-[11px] text-muted-foreground">Timeout (seconds)</label>
        <input
          type="number"
          min={1}
          value={timeout}
          onChange={(e) => setTimeoutVal(e.target.value)}
          className="rounded-sm border border-border bg-background px-2 py-1 text-[13px] outline-none focus:border-primary"
        />
      </div>

      {/* Streaming toggle */}
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          id="config-streaming"
          checked={streaming}
          onChange={(e) => setStreaming(e.target.checked)}
          className="size-3"
        />
        <label htmlFor="config-streaming" className="text-[13px]">
          Streaming
        </label>
      </div>

      {/* Sensitive field notice */}
      <div className="rounded-sm border border-border bg-muted/50 p-2 text-[11px] text-muted-foreground">
        API key is managed via settings.json or environment variables — not
        editable here for security.
      </div>

      {/* Save button */}
      <button
        type="button"
        className="rounded-sm bg-primary px-3 py-1 text-[12px] text-primary-foreground disabled:opacity-50"
        onClick={save}
        disabled={saving || !hasChanges}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  );
}
