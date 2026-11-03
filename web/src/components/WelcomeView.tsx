import { useEffect, useState } from "react";
import { FolderOpen, MessageSquarePlus } from "lucide-react";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";
import type { ProjectInfo } from "../api/types";
import { Button } from "./ui/button";
import { SessionsBrowserModal } from "./SessionsBrowserModal";

const CONNECTION_LABEL: Record<string, string> = {
  unknown: "unknown",
  connected: "connected",
  disconnected: "disconnected",
};

const CONNECTION_DOT: Record<string, string> = {
  unknown: "bg-muted-foreground",
  connected: "bg-success",
  disconnected: "bg-danger",
};

/**
 * Empty-state landing page shown when there is no active session (fresh launch
 * with no restorable history, or after the last tab closes). The welcome
 * screen replaces the chat pane; picking any action opens a session and the
 * normal shell takes over.
 */
export function WelcomeView({ client }: { client: DaemonClient }) {
  const [browsing, setBrowsing] = useState(false);
  const connection = useSessionManager((s) => s.connection);
  const [noProject, setNoProject] = useState<ProjectInfo | null>(null);

  // Resolve the daemon's synthetic No Project workspace once, so welcome-page
  // sessions live there (never silently attached to the main project). Falls
  // back to the legacy main-project behavior while offline/undetermined.
  useEffect(() => {
    let cancelled = false;
    client
      .listProjects()
      .then((ps) => {
        if (!cancelled) setNoProject(ps.find((p) => p.is_no_project) ?? null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [client]);

  const newSession = async () => {
    if (!noProject) {
      // Daemon unreachable or pre-feature daemon: legacy local session that
      // attaches to the main project on first run.
      useSessionManager.getState().createLocalSession();
      return;
    }
    try {
      const created = await client.createSession({
        name: undefined,
        project_path: noProject.path,
      });
      useSessionManager.getState().createLocalSession(undefined, {
        id: created.id,
        daemonId: created.id,
        projectPath: noProject.path,
      });
    } catch {
      // Creation failed (daemon hiccup) — still open a local session so the
      // user is never stranded on the welcome screen.
      useSessionManager.getState().createLocalSession();
    }
  };

  return (
    <div className="flex h-full flex-col items-center justify-center overflow-y-auto p-6">
      <div className="flex w-full max-w-sm flex-col items-center gap-4 rounded-lg border border-border bg-card p-8 text-center shadow-sm">
        <div className="flex h-12 w-12 items-center justify-center rounded-md bg-primary text-[15px] font-bold text-primary-foreground">
          WG
        </div>
        <h1 className="text-lg font-semibold text-foreground">Wgenty Code</h1>
        <p className="text-[13px] text-muted-foreground">
          Command center for your coding agent. Start a session to talk to the daemon in this
          workspace.
        </p>

        <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <span className={`h-1.5 w-1.5 rounded-full ${CONNECTION_DOT[connection] ?? ""}`} />
          Daemon {CONNECTION_LABEL[connection] ?? connection}
        </div>

        <div className="mt-2 flex flex-col gap-2 self-stretch">
          <Button onClick={newSession}>
            <MessageSquarePlus size={15} /> New session
          </Button>
          <Button variant="outline" onClick={() => setBrowsing(true)}>
            <FolderOpen size={15} /> Open saved session
          </Button>
        </div>

        <p className="mt-2 text-[12px] text-muted-foreground">
          Inside a session,{" "}
          <kbd className="rounded border border-border bg-muted px-1 font-mono text-[11px]">
            /model
          </kbd>{" "}
          switches models and{" "}
          <kbd className="rounded border border-border bg-muted px-1 font-mono text-[11px]">
            /sessions
          </kbd>{" "}
          browses history.
        </p>
      </div>

      {browsing && <SessionsBrowserModal client={client} onClose={() => setBrowsing(false)} />}
    </div>
  );
}
