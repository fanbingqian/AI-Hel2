import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useChatStore } from "./chatStore";

export type VoiceMode = "text" | "voice";
export type VoiceStatus = "idle" | "listening" | "transcribing" | "preview" | "sending";
export type TtsStatus = "idle" | "speaking" | "interrupted";
export type VoiceSource = "chat" | "sphere";

export interface VoiceState {
  inputMode: VoiceMode;
  status: VoiceStatus;
  duration: number;
  transcribedText: string;
  ttsStatus: TtsStatus;
  spectrum: Float32Array;
  volume: number;
  error: string | null;
  voiceSource: VoiceSource;

  // Backward-compatible aliases
  isListening: boolean;
  depsChecking: boolean;
  voiceText: string;

  // Actions
  toggleInputMode: () => void;
  startRecording: (source: VoiceSource) => Promise<void>;
  stopRecording: () => Promise<void>;
  cancelRecording: () => void;
  confirmAndSend: (text?: string) => void;
  setTtsStatus: (status: TtsStatus) => void;
  setSpectrum: (data: Float32Array) => void;
  setVolume: (v: number) => void;

  // Backward-compatible actions
  startListening: (source: VoiceSource) => Promise<void>;
  stopListening: () => Promise<void>;
  listenOnce: (source: VoiceSource) => Promise<void>;
  clearVoiceText: () => void;
  clearError: () => void;
}

// Helper: set status + sync backward-compat fields
function setStatus(set: any, status: VoiceStatus, extra?: Record<string, unknown>) {
  set({ status, isListening: status === "listening", ...extra });
}

let depsChecked = false;

async function ensureDeps(): Promise<boolean> {
  if (depsChecked) return true;
  depsChecked = true;
  useVoiceStore.setState({ depsChecking: true, error: null });
  try {
    const missing: string[] = await invoke("check_voice_deps");
    if (missing.length > 0) {
      useVoiceStore.setState({
        error: `语音依赖缺失: ${missing.join(", ")}`,
        depsChecking: false,
      });
      return false;
    }
    await invoke("prewarm_voice_model");
    try {
      const diag: string = await invoke("voice_diagnose");
      console.log("[Voice] Diagnostic:\n" + diag);
    } catch { /* optional */ }
  } catch (e) {
    const msg = typeof e === "string" ? e : (e as Error).message || "语音准备失败";
    useVoiceStore.setState({ error: msg, depsChecking: false });
    return false;
  }
  useVoiceStore.setState({ depsChecking: false });
  return true;
}

export const useVoiceStore = create<VoiceState>((set, get) => ({
  inputMode: "text",
  status: "idle",
  duration: 0,
  transcribedText: "",
  ttsStatus: "idle",
  spectrum: new Float32Array(64),
  volume: 0,
  error: null,
  voiceSource: "chat",
  isListening: false,
  depsChecking: false,
  voiceText: "",

  toggleInputMode: () =>
    set((s) => ({ inputMode: s.inputMode === "text" ? "voice" : "text" })),

  startRecording: async (source) => {
    const { status } = get();
    if (status !== "idle") return;
    // Set listening immediately so stopRecording/cancelRecording can see it
    setStatus(set, "listening", { error: null, voiceSource: source, transcribedText: "", voiceText: "" });
    const ok = await ensureDeps();
    if (!ok) {
      setStatus(set, "idle");
      return;
    }
    try {
      await invoke("voice_start_listening");
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error).message || "语音启动失败";
      setStatus(set, "idle", { error: msg });
    }
  },

  stopRecording: async () => {
    const { status } = get();
    if (status !== "listening") return;
    setStatus(set, "transcribing");
    try {
      const text: string = await invoke("voice_stop_listening");
      const trimmed = text.trim();
      if (trimmed) {
        setStatus(set, "preview", { transcribedText: trimmed, voiceText: trimmed });
      } else {
        setStatus(set, "idle", { error: "未检测到语音" });
      }
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error).message || "语音识别失败";
      setStatus(set, "idle", { error: msg });
    }
  },

  cancelRecording: () => {
    if (get().status === "listening") {
      invoke("voice_stop_listening").catch(() => {});
    }
    setStatus(set, "idle", { transcribedText: "", voiceText: "", error: null });
  },

  confirmAndSend: (text) => {
    const msg = text || get().transcribedText;
    if (!msg) return;
    useChatStore.getState().sendMessage(msg);
    setStatus(set, "idle", { transcribedText: "", voiceText: "" });
  },

  setTtsStatus: (ttsStatus) => set({ ttsStatus }),
  setSpectrum: (data) => set({ spectrum: data }),
  setVolume: (volume) => set({ volume }),

  // ── Backward-compatible actions ──
  startListening: async (source) => {
    get().startRecording(source);
  },

  stopListening: async () => {
    if (get().status !== "listening") return;
    await get().stopRecording();
    // KnowledgeSphere: auto-send after transcription
    if (get().status === "preview" && get().transcribedText) {
      get().confirmAndSend(get().transcribedText);
    }
  },

  listenOnce: async (source) => {
    const { status } = get();
    if (status !== "idle") return;
    setStatus(set, "listening", { error: null, voiceSource: source, transcribedText: "", voiceText: "" });
    const ok = await ensureDeps();
    if (!ok) {
      setStatus(set, "idle");
      return;
    }
    try {
      const text: string = await invoke("voice_listen_once");
      const trimmed = text.trim();
      if (trimmed) {
        setStatus(set, "preview", { transcribedText: trimmed, voiceText: trimmed });
      } else {
        setStatus(set, "idle", { error: "未检测到语音" });
      }
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error).message || "语音识别失败";
      setStatus(set, "idle", { error: msg });
    }
  },

  clearVoiceText: () => set({ transcribedText: "", voiceText: "" }),
  clearError: () => set({ error: null }),
}));
