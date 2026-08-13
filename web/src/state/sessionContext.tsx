/**
 * Per-session store injection. CenterPane wraps the active session's UI in a
 * Provider; components inside subscribe via `useSessionStore` instead of a
 * global singleton — which is what makes concurrent sessions possible.
 */
import { createContext, useContext } from "react";
import { useStore } from "zustand";
import type { SessionState, SessionStore } from "./sessionStore";

export const SessionStoreContext = createContext<SessionStore | null>(null);

export function useSessionStore<T>(selector: (s: SessionState) => T): T {
  const store = useContext(SessionStoreContext);
  if (!store) throw new Error("useSessionStore must be used inside SessionStoreContext.Provider");
  return useStore(store, selector);
}
