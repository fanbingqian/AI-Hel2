import { create } from "zustand";
import type { UserInfo } from "../types";
import { getConfig, getCurrentUser } from "../services/api";

export type AuthStage = "splash" | "login" | "register" | "api_setup" | "done";

interface AuthState {
  stage: AuthStage;
  user: UserInfo | null;
  isFirstRun: boolean;
  loading: boolean;
  error: string | null;
  setStage: (stage: AuthStage) => void;
  setUser: (user: UserInfo | null) => void;
  setError: (error: string | null) => void;
  checkFirstRun: () => Promise<void>;
  proceedAfterLogin: () => Promise<void>;
  proceedAfterRegister: () => void;
  logout: () => void;
  resetAuth: () => void;
  proceedAfterSplash: () => void;
}

async function isApiConfigured(): Promise<boolean> {
  try {
    const config = await getConfig();
    const model = (config as any)?.model;
    if (!model) return false;
    return !!model.api_key;
  } catch {
    return false;
  }
}

export const useAuthStore = create<AuthState>((set, get) => ({
  stage: "splash",
  user: null,
  isFirstRun: false,
  loading: false,
  error: null,

  setStage: (stage) => set({ stage }),

  setUser: (user) => set({ user, error: null }),

  setError: (error) => set({ error }),

  checkFirstRun: async () => {
    set({ loading: true, error: null });
    try {
      const user = await getCurrentUser() as any;
      if (user?.name) {
        set({ isFirstRun: false, stage: "login", loading: false });
      } else {
        set({ isFirstRun: true, stage: "register", loading: false });
      }
    } catch {
      set({ isFirstRun: true, stage: "register", loading: false });
    }
  },

  proceedAfterLogin: async () => {
    const configured = await isApiConfigured();
    if (configured) {
      set({ stage: "done" });
    } else {
      set({ stage: "api_setup" });
    }
  },

  proceedAfterRegister: () => {
    set({ stage: "api_setup" });
  },

  logout: () => {
    set({ user: null, stage: "login", isFirstRun: false });
  },

  resetAuth: () => {
    set({ user: null, stage: "login", isFirstRun: false, error: null });
  },

  proceedAfterSplash: () => {
    get().checkFirstRun();
  },
}));
