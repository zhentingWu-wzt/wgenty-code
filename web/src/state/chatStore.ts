/**
 * Back-compat singleton. Pre-command-center components still import
 * `useChatStore`; Task 7 migrates them to `useSessionStore` (context) and
 * deletes this file.
 */
import { createSessionStore } from "./sessionStore";

export const useChatStore = createSessionStore();
export type { DisplayMessage, ConnectionStatus, TurnError, SessionState } from "./sessionStore";
