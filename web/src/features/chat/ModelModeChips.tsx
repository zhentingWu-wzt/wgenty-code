import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { DaemonClient } from "../../api/client";
import type { PermissionMode, ProviderGroupInfo } from "../../api/types";
import { useSessionManager, type SessionEntry } from "../../state/sessionManager";

const MODE_LABELS: Record<PermissionMode, string> = {
  normal: "normal",
  accept_edits: "accept edits",
  yolo: "yolo",
};
const MODE_ORDER: PermissionMode[] = ["normal", "accept_edits", "yolo"];

export function ModelModeChips({ client }: { client: DaemonClient }) {
  const entry = useSessionManager((state) =>
    state.activeId ? (state.entries[state.activeId] ?? null) : null,
  );
  if (!entry) return null;
  const sessionId = entry.daemonId ?? entry.id;
  return <ActiveModelModeChips key={`${entry.id}:${sessionId}`} client={client} entry={entry} />;
}

function ActiveModelModeChips({ client, entry }: { client: DaemonClient; entry: SessionEntry }) {
  const entryId = entry.id;
  const sessionId = entry.daemonId ?? entry.id;
  const [open, setOpen] = useState<"model" | "mode" | null>(null);
  const [providers, setProviders] = useState<ProviderGroupInfo[]>([]);
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [loadingModels, setLoadingModels] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const catalogGeneration = useRef(0);
  const mounted = useRef(true);
  const isRunning = useSyncExternalStore(
    entry.store.subscribe,
    () => entry.store.getState().isRunning,
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      catalogGeneration.current += 1;
    };
  }, []);

  const closePopovers = () => {
    catalogGeneration.current += 1;
    setOpen(null);
    setSelectedProvider(null);
    setError(null);
  };

  const loadModels = async () => {
    const generation = ++catalogGeneration.current;
    setProviders([]);
    setSelectedProvider(null);
    setLoadingModels(true);
    setError(null);
    try {
      const response = await client.listModels(sessionId);
      if (mounted.current && catalogGeneration.current === generation) {
        setProviders(response.providers);
      }
    } catch (cause) {
      if (mounted.current && catalogGeneration.current === generation) {
        setError(`Model list failed: ${messageOf(cause)}. Try again.`);
      }
    } finally {
      if (mounted.current && catalogGeneration.current === generation) {
        setLoadingModels(false);
      }
    }
  };

  const openModels = () => {
    if (open === "model") {
      closePopovers();
      return;
    }
    setOpen("model");
    void loadModels();
  };

  const openModes = () => {
    catalogGeneration.current += 1;
    setSelectedProvider(null);
    setError(null);
    setOpen((current) => (current === "mode" ? null : "mode"));
  };

  const chooseModel = async (provider: string, model: string) => {
    setPending(true);
    setError(null);
    try {
      const response = await client.switchModel({ provider, model, sessionId });
      useSessionManager.getState().setSessionModel(entryId, {
        provider,
        model: response.model_name,
        isOverride: true,
      });
      if (mounted.current) closePopovers();
    } catch (cause) {
      if (mounted.current) setError(`Model switch failed: ${messageOf(cause)}. Try again.`);
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  const resetModel = async () => {
    setPending(true);
    setError(null);
    try {
      await client.resetSessionModel(sessionId);
      if (mounted.current) closePopovers();
    } catch (cause) {
      if (mounted.current) setError(`Model reset failed: ${messageOf(cause)}. Try again.`);
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  const chooseMode = async (mode: PermissionMode) => {
    setPending(true);
    setError(null);
    try {
      const response = await client.setPermissionMode(sessionId, mode);
      useSessionManager.getState().setSessionMode(entryId, {
        mode: response.mode,
        isOverride: response.is_override === true,
      });
      if (mounted.current) closePopovers();
    } catch (cause) {
      if (mounted.current) setError(`Mode switch failed: ${messageOf(cause)}. Try again.`);
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  const resetMode = async () => {
    setPending(true);
    setError(null);
    try {
      const response = await client.resetSessionMode(sessionId);
      useSessionManager.getState().setSessionMode(entryId, {
        mode: response.mode,
        isOverride: response.is_override === true,
      });
      if (mounted.current) closePopovers();
    } catch (cause) {
      if (mounted.current) setError(`Mode reset failed: ${messageOf(cause)}. Try again.`);
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  const modelLabel = entry.model
    ? entry.model.provider
      ? `${entry.model.provider}/${entry.model.model}`
      : entry.model.model
    : "-";
  const modelAccessibleLabel = `Session model: ${modelLabel}${entry.model?.isOverride ? " (session override)" : ""}`;
  const modeAccessibleLabel = `Permission mode: ${entry.mode?.mode ?? "-"}${entry.mode?.isOverride ? " (session override)" : ""}`;
  const provider = providers.find((candidate) => candidate.key === selectedProvider) ?? null;

  return (
    <div
      className="flex min-h-7 shrink-0 items-center gap-2 border-t border-border bg-background px-3 py-1 text-[11px] text-muted-foreground"
      aria-label="Session model and permission mode"
    >
      {!isRunning && (
        <div className="relative min-w-0">
          <button
            type="button"
            aria-label={modelAccessibleLabel}
            aria-expanded={open === "model"}
            aria-haspopup="menu"
            onClick={openModels}
            className="flex max-w-full items-center gap-1.5 rounded-sm border border-border px-2 py-0.5 font-mono hover:bg-accent"
          >
            <span className="truncate">{modelLabel}</span>
            {entry.model?.isOverride && (
              <span className="shrink-0 rounded-sm bg-accent px-1 text-[9px] text-foreground">
                override
              </span>
            )}
          </button>
          {open === "model" && (
            <>
              <PopoverDismiss label="Close model picker" onClose={closePopovers} />
              <div className="absolute bottom-full left-0 z-50 mb-1 max-h-80 w-72 overflow-y-auto rounded-md border border-border bg-popover py-1 text-foreground shadow-lg">
                {error && (
                  <div role="alert" className="px-2 py-1 text-danger">
                    {error}
                  </div>
                )}
                {loadingModels ? (
                  <div className="px-2 py-1 text-muted-foreground">Loading models…</div>
                ) : provider ? (
                  <>
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedProvider(null);
                        setError(null);
                      }}
                      className="w-full px-2 py-1 text-left text-muted-foreground hover:bg-accent"
                    >
                      ← Providers
                    </button>
                    {provider.models.map((model) => (
                      <button
                        key={model.name}
                        type="button"
                        disabled={pending || model.active}
                        onClick={() => void chooseModel(provider.key, model.name)}
                        className="flex w-full items-center gap-2 px-2 py-1 text-left hover:bg-accent disabled:opacity-60"
                      >
                        <span className="min-w-0 flex-1 truncate font-mono">{model.name}</span>
                        {model.tier && <span className="text-muted-foreground">{model.tier}</span>}
                        {model.active && <span className="text-success">active</span>}
                      </button>
                    ))}
                  </>
                ) : (
                  <>
                    {entry.model?.isOverride && (
                      <button
                        type="button"
                        disabled={pending}
                        onClick={() => void resetModel()}
                        className="w-full border-b border-border px-2 py-1 text-left hover:bg-accent disabled:opacity-60"
                      >
                        Follow default
                      </button>
                    )}
                    {!loadingModels && error && (
                      <button
                        type="button"
                        onClick={() => void loadModels()}
                        className="w-full px-2 py-1 text-left hover:bg-accent"
                      >
                        Retry
                      </button>
                    )}
                    {providers.map((group) => (
                      <button
                        key={group.key}
                        type="button"
                        onClick={() => {
                          setSelectedProvider(group.key);
                          setError(null);
                        }}
                        className="flex w-full items-center justify-between px-2 py-1 text-left hover:bg-accent"
                      >
                        <span>{group.display_name}</span>
                        <span aria-hidden>›</span>
                      </button>
                    ))}
                    {!error && providers.length === 0 && (
                      <div className="px-2 py-1 text-muted-foreground">No models configured</div>
                    )}
                  </>
                )}
              </div>
            </>
          )}
        </div>
      )}

      <div className="relative">
        <button
          type="button"
          aria-label={modeAccessibleLabel}
          aria-expanded={open === "mode"}
          aria-haspopup="menu"
          onClick={openModes}
          className="flex items-center gap-1.5 rounded-sm border border-border px-2 py-0.5 hover:bg-accent"
        >
          <span>{entry.mode ? MODE_LABELS[entry.mode.mode] : "-"}</span>
          {entry.mode?.isOverride && (
            <span className="rounded-sm bg-accent px-1 text-[9px] text-foreground">override</span>
          )}
        </button>
        {open === "mode" && (
          <>
            <PopoverDismiss label="Close mode picker" onClose={closePopovers} />
            <div className="absolute bottom-full left-0 z-50 mb-1 w-44 rounded-md border border-border bg-popover py-1 text-foreground shadow-lg">
              {error && (
                <div role="alert" className="px-2 py-1 text-danger">
                  {error}
                </div>
              )}
              {entry.mode?.isOverride && (
                <button
                  type="button"
                  disabled={pending}
                  onClick={() => void resetMode()}
                  className="w-full border-b border-border px-2 py-1 text-left hover:bg-accent disabled:opacity-60"
                >
                  Follow project default
                </button>
              )}
              {MODE_ORDER.map((mode) => (
                <button
                  key={mode}
                  type="button"
                  disabled={pending}
                  onClick={() => void chooseMode(mode)}
                  className="flex w-full items-center justify-between px-2 py-1 text-left hover:bg-accent disabled:opacity-60"
                >
                  <span>{MODE_LABELS[mode]}</span>
                  {entry.mode?.mode === mode && <span className="text-success">✓</span>}
                </button>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function PopoverDismiss({ label, onClose }: { label: string; onClose: () => void }) {
  return (
    <button
      type="button"
      aria-label={label}
      className="fixed inset-0 z-40 cursor-default"
      onClick={onClose}
    />
  );
}

function messageOf(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
