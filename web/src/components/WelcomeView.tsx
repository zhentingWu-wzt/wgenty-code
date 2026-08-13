import { useState } from "react";
import { FolderOpen, MessageSquarePlus } from "lucide-react";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";
import { SessionsBrowserModal } from "./SessionsBrowserModal";

/**
 * Empty-state landing page shown when there are no open sessions (fresh
 * launch before the first "New session" click). Replaces the old behavior of
 * silently auto-creating a throwaway local session on load: the command
 * center now boots into a welcome screen with explicit actions.
 */
export function WelcomeView({ client }: { client: DaemonClient }) {
  const [browsing, setBrowsing] = useState(false);
  const connection = useSessionManager((s) => s.connection);

  const newSession = () => {
    useSessionManager.getState().createLocalSession();
  };

  return (
    <div className="welcome">
      <div className="welcome-card">
        <div className="welcome-logo">WG</div>
        <h1 className="welcome-title">Wgenty Code</h1>
        <p className="welcome-subtitle">
          Command center for your coding agent. Start a session to talk to the
          daemon in this workspace.
        </p>

        <div className={`welcome-status welcome-status-${connection}`}>
          <span className="welcome-status-dot" />
          Daemon {connection}
        </div>

        <div className="welcome-actions">
          <button type="button" className="btn btn-primary" onClick={newSession}>
            <MessageSquarePlus size={15} /> New session
          </button>
          <button type="button" className="btn" onClick={() => setBrowsing(true)}>
            <FolderOpen size={15} /> Open saved session
          </button>
        </div>

        <p className="welcome-hint">
          Inside a session, <kbd>/model</kbd> switches models and{" "}
          <kbd>/sessions</kbd> browses history.
        </p>
      </div>

      {browsing && (
        <SessionsBrowserModal client={client} onClose={() => setBrowsing(false)} />
      )}
    </div>
  );
}
