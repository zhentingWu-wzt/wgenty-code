import { useCallback, useEffect, useState } from "react";
import { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";
import { cn } from "../../lib/utils";

interface ModelOption {
  key: string;
  provider: string;
  label: string;
  model_name: string;
  tier?: "light" | "medium" | "heavy" | null;
  active: boolean;
}

/** Global model picker via the v2 provider-grouped catalog. Session-scoped
 *  selection is handled by the active-session controls, not this modal. */
export function ModelPanel({ client }: { client: DaemonClient }) {
  const [models, setModels] = useState<ModelOption[]>([]);
  const [loading, setLoading] = useState(true);
  const applyModelChanged = useSessionManager((s) => s.applyModelChanged);
  const [switching, setSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listModels()
      .then((res) => {
        setModels(
          res.providers.flatMap((group) =>
            group.models.map((model) => ({
              key: `${group.key}/${model.name}`,
              provider: group.key,
              label: group.display_name,
              model_name: model.name,
              tier: model.tier,
              active: model.active,
            })),
          ),
        );
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [client]);

  useEffect(refresh, [refresh]);

  const onSwitch = async (option: ModelOption) => {
    setSwitching(option.key);
    setError(null);
    try {
      const res = await client.switchModel({
        provider: option.provider,
        model: option.model_name,
      });
      applyModelChanged({ provider: res.provider ?? option.provider, model_name: res.model_name });
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
    return (
      <div className="flex flex-col gap-2 p-2 text-[12px] text-muted-foreground">
        <div>
          No models configured. Add{" "}
          <span className="font-mono text-foreground">models.providers</span> in{" "}
          <span className="font-mono text-foreground">~/.wgenty-code/settings.json</span> to enable
          switching, e.g.:
        </div>
        <pre className="overflow-x-auto rounded-md border border-border bg-background p-2 font-mono text-[11px] text-foreground">{`"models": {
  "providers": {
    "zhipu": {
      "base_url": "https://…",
      "models": [{ "name": "glm-5.3" }]
    }
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
              onClick={() => onSwitch(m)}
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
