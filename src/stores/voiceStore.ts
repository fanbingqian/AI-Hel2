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
    // Whisper doesn't need pre-warming
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

  // Toggle input mode; eagerly check voice deps when switching to voice mode
  // so that startRecording doesn't have a slow first-call path.
  toggleInputMode: () => {
    const next = get().inputMode === "text" ? "voice" : "text";
    set({ inputMode: next });
    if (next === "voice") {
      ensureDeps();
    }
  },

  // Start recording — now fast because deps are checked eagerly.
  // The Rust backend spawns a background recording thread that runs until
  // stop_ptt_recording signals it to stop.
  startRecording: async (source) => {
    const { status } = get();
    if (status !== "idle") return;
    setStatus(set, "listening", { error: null, voiceSource: source, transcribedText: "", voiceText: "" });
    // Only awaits deps on the very first call (cold path); subsequent calls skip.
    if (!depsChecked) {
      const ok = await ensureDeps();
      if (!ok) {
        setStatus(set, "idle");
        return;
      }
    }
    try {
      await invoke("start_ptt_recording");
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
      const text: string = await invoke("stop_ptt_recording");
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
      invoke("cancel_ptt_recording").catch(() => {});
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
    if (!depsChecked) {
      const ok = await ensureDeps();
      if (!ok) {
        setStatus(set, "idle");
        return;
      }
    }
    try {
      await invoke("start_ptt_recording");
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error).message || "语音识别失败";
      setStatus(set, "idle", { error: msg });
    }
  },

  clearVoiceText: () => set({ transcribedText: "", voiceText: "" }),
  clearError: () => set({ error: null }),
}));
