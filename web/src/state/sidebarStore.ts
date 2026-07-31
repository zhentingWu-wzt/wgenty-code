/**
 * Sidebar store — owns the side panel's tab selection, collapse state, and the
 * polled data for the read-only panels (todos, tasks, models, sessions).
 *
 * Kept separate from chatStore so chat re-renders don't drag panel data along
 * and vice versa. Polling is driven by the panels themselves via useEffect;
 * this store just holds the latest snapshot + an epoch counter components can
 * subscribe to for forced refreshes.
 */
import { create } from "zustand";
import type {
  GetTodosResponse,
  ListTasksResponse,
  ModelOption,
  SessionInfo,
  TaskProgressResponse,
} from "../api/types";

export type SidebarTab = "sessions" | "todos" | "tasks" | "model" | "memory" | "config";

interface SidebarState {
  collapsed: boolean;
  activeTab: SidebarTab;
  // Polled snapshots (undefined = not loaded yet).
  sessions: SessionInfo[];
  todos: GetTodosResponse | null;
  tasks: ListTasksResponse | null;
  taskProgress: TaskProgressResponse | null;
  models: ModelOption[];

  toggleCollapsed: () => void;
  setActiveTab: (tab: SidebarTab) => void;
  setSessions: (s: SessionInfo[]) => void;
  setTodos: (t: GetTodosResponse | null) => void;
  setTasks: (t: ListTasksResponse | null) => void;
  setTaskProgress: (t: TaskProgressResponse | null) => void;
  setModels: (m: ModelOption[]) => void;
}

export const useSidebarStore = create<SidebarState>((set) => ({
  collapsed: false,
  activeTab: "sessions",
  sessions: [],
  todos: null,
  tasks: null,
  taskProgress: null,
  models: [],

  toggleCollapsed: () => set((s) => ({ collapsed: !s.collapsed })),
  setActiveTab: (tab) => set({ activeTab: tab, collapsed: false }),
  setSessions: (sessions) => set({ sessions }),
  setTodos: (todos) => set({ todos }),
  setTasks: (tasks) => set({ tasks }),
  setTaskProgress: (taskProgress) => set({ taskProgress }),
  setModels: (models) => set({ models }),
}));
