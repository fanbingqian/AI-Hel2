import { create } from "zustand";
import type { UserInfo } from "../types";

interface SettingsState {
  user: UserInfo | null;
  theme: "dark" | "light" | "system";
  language: "zh-CN" | "en";
  isLoggedIn: boolean;
  ttsSpeaker: number;
  ttsEnabled: boolean;
  setUser: (user: UserInfo | null) => void;
  setTtsSpeaker: (id: number) => void;
  setTtsEnabled: (enabled: boolean) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  user: null,
  theme: "dark",
  language: "zh-CN",
  isLoggedIn: false,
  ttsSpeaker: 0,
  ttsEnabled: false,
  setUser: (user) => set({ user, isLoggedIn: !!user }),
  setTtsSpeaker: (ttsSpeaker) => set({ ttsSpeaker }),
  setTtsEnabled: (ttsEnabled) => set({ ttsEnabled }),
}));
