import { create } from "zustand";
import type { PageId } from "../types";

export interface AgentStatus {
  running: boolean;
  pid: number | null;
  port: number;
  version: string | null;
  healthy: boolean;
  error: string | null;
}

interface UIState {
  activePage: PageId;
  setActivePage: (page: PageId) => void;
  sessionListWidth: number;
  setSessionListWidth: (w: number) => void;
  chatPanelWidth: number;
  setChatPanelWidth: (w: number) => void;
  sessionListExpanded: boolean;
  toggleSessionList: () => void;
  agentStatus: AgentStatus | null;
  setAgentStatus: (s: AgentStatus | null) => void;
  showAgentLogs: boolean;
  setShowAgentLogs: (v: boolean) => void;
}

export const useUIStore = create<UIState>((set) => ({
  activePage: "sphere",
  setActivePage: (page) => set({ activePage: page }),

  sessionListWidth: 210,
  setSessionListWidth: (w) =>
    set({ sessionListWidth: Math.min(360, Math.max(160, w)) }),

  chatPanelWidth: 310,
  setChatPanelWidth: (w) =>
    set({ chatPanelWidth: Math.min(500, Math.max(220, w)) }),

  sessionListExpanded: true,
  toggleSessionList: () =>
    set((s) => ({ sessionListExpanded: !s.sessionListExpanded })),

  agentStatus: null,
  setAgentStatus: (agentStatus) => set({ agentStatus }),
  showAgentLogs: false,
  setShowAgentLogs: (showAgentLogs) => set({ showAgentLogs }),
}));
