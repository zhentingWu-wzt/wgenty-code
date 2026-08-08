import { useCallback, useState } from "react";
import { Play, Plus, Square, Trash2 } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { McpServerInfo } from "../../api/types";
import { usePolling } from "../../hooks/usePolling";

/**
 * MCP servers panel: list servers with status/tools, start/stop/remove actions,
 * and an inline add form. Uses the daemon's MCP management API
 * (GET/POST /mcp/servers, POST /mcp/servers/:name/{start,stop}, DELETE /mcp/servers/:name).
 */
export function McpPanel({ client }: { client: DaemonClient }) {
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null); // server name being operated on

  // Add form state
  const [showAdd, setShowAdd] = useState(false);
  const [newName, setNewName] = useState("");
  const [newCommand, setNewCommand] = useState("");
  const [newArgs, setNewArgs] = useState("");

  const refresh = useCallback(async () => {
    try {
      const s = await client.listMcpServers();
      setServers(s);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  usePolling(refresh, true, 10000);

  const wrap = async (name: string, fn: () => Promise<void>) => {
    setBusy(name);
    setError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const onAdd = async () => {
    if (!newName.trim() || !newCommand.trim()) return;
    await wrap("__add__", async () => {
      await client.addMcpServer({
        name: newName.trim(),
        command: newCommand.trim(),
        args: newArgs.trim() ? newArgs.trim().split(/\s+/) : [],
        auto_start: true,
      });
      setNewName("");
      setNewCommand("");
      setNewArgs("");
      setShowAdd(false);
    });
  };

  const statusColor = (status: string) => {
    const s = status.toLowerCase();
    if (s === "running") return "text-green-500";
    if (s === "stopped") return "text-muted-foreground";
    if (s === "error") return "text-danger";
    return "text-muted-foreground";
  };

  return (
    <div className="flex flex-col gap-2 p-2">
      {error && <div className="p-2 text-danger">{error}</div>}

      {/* Add button / form */}
      {showAdd ? (
        <div className="flex flex-col gap-1 rounded-sm border border-border p-2">
          <input
            type="text"
            placeholder="name"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            className="rounded-sm border border-border bg-background px-2 py-1 text-[12px] outline-none focus:border-primary"
          />
          <input
            type="text"
            placeholder="command (e.g. npx)"
            value={newCommand}
            onChange={(e) => setNewCommand(e.target.value)}
            className="rounded-sm border border-border bg-background px-2 py-1 text-[12px] outline-none focus:border-primary"
          />
          <input
            type="text"
            placeholder="args (space-separated)"
            value={newArgs}
            onChange={(e) => setNewArgs(e.target.value)}
            className="rounded-sm border border-border bg-background px-2 py-1 text-[12px] outline-none focus:border-primary"
          />
          <div className="flex gap-2">
            <button
              type="button"
              className="rounded-sm bg-primary px-2 py-0.5 text-[11px] text-primary-foreground"
              onClick={onAdd}
              disabled={busy === "__add__"}
            >
              Add
            </button>
            <button
              type="button"
              className="rounded-sm border border-border px-2 py-0.5 text-[11px]"
              onClick={() => setShowAdd(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          className="flex items-center gap-1 rounded-sm border border-border px-2 py-1 text-[12px] hover:bg-accent"
          onClick={() => setShowAdd(true)}
        >
          <Plus size={12} /> Add server
        </button>
      )}

      {/* Server list */}
      {servers.length === 0 ? (
        <div className="p-2 text-[12px] text-muted-foreground">No MCP servers configured.</div>
      ) : (
        <ul className="flex flex-col gap-1">
          {servers.map((srv) => {
            const isRunning = srv.status.toLowerCase() === "running";
            return (
              <li key={srv.name} className="flex items-center gap-1 rounded-sm border border-border p-2">
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[13px]">{srv.name}</span>
                  <span className={`text-[11px] ${statusColor(srv.status)}`}>
                    {srv.status} · {srv.tools_count} tools
                  </span>
                </div>
                <button
                  type="button"
                  className="shrink-0 rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                  title={isRunning ? "Stop" : "Start"}
                  disabled={busy === srv.name}
                  onClick={() =>
                    wrap(srv.name, () =>
                      isRunning ? client.stopMcpServer(srv.name) : client.startMcpServer(srv.name),
                    )
                  }
                >
                  {isRunning ? <Square size={12} /> : <Play size={12} />}
                </button>
                <button
                  type="button"
                  className="shrink-0 rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-danger disabled:opacity-50"
                  title="Remove"
                  disabled={busy === srv.name}
                  onClick={() =>
                    window.confirm(`Remove MCP server "${srv.name}"?`) &&
                    wrap(srv.name, () => client.removeMcpServer(srv.name))
                  }
                >
                  <Trash2 size={12} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
