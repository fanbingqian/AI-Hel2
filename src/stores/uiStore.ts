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

export type MainContentMode = "graph2d" | "editor" | "canvas" | "entity" | "preview";

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
  // AI Hel2 layout
  panelCollapsed: boolean;
  setPanelCollapsed: (v: boolean) => void;
  mainContentMode: MainContentMode;
  setMainContentMode: (m: MainContentMode) => void;
  openFilePath: string | null;
  setOpenFilePath: (p: string | null) => void;
  docTreeWidth: number;
  setDocTreeWidth: (w: number) => void;
  mainContentWidth: number;
  setMainContentWidth: (w: number) => void;
}

export const useUIStore = create<UIState>((set) => ({
  activePage: "aihel",
  setActivePage: (page) => set({ activePage: page }),

  sessionListWidth: 210,
  setSessionListWidth: (w) =>
    set({ sessionListWidth: Math.min(360, Math.max(160, w)) }),

  chatPanelWidth: 390,
  setChatPanelWidth: (w) =>
    set({ chatPanelWidth: Math.min(500, Math.max(340, w)) }),

  sessionListExpanded: true,
  toggleSessionList: () =>
    set((s) => ({ sessionListExpanded: !s.sessionListExpanded })),

  agentStatus: null,
  setAgentStatus: (agentStatus) => set({ agentStatus }),
  showAgentLogs: false,
  setShowAgentLogs: (showAgentLogs) => set({ showAgentLogs }),

  panelCollapsed: true,
  setPanelCollapsed: (panelCollapsed) => set({ panelCollapsed }),

  mainContentMode: "graph2d",
  setMainContentMode: (mainContentMode) => set({ mainContentMode }),

  openFilePath: null,
  setOpenFilePath: (openFilePath) => set({ openFilePath }),

  docTreeWidth: 260,
  setDocTreeWidth: (w) =>
    set({ docTreeWidth: Math.min(400, Math.max(180, w)) }),

  mainContentWidth: 0,
  setMainContentWidth: (w) =>
    set({ mainContentWidth: Math.min(900, Math.max(200, w)) }),
}));
