// ── Shared types ──

export type PageId = "aihel" | "aiword" | "settings";
export type AuthStage = "splash" | "login" | "register" | "api_setup" | "done";

// ── Chat ──
export interface ChatMessage {
  id: string;
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface StreamDelta {
  messageId: string;
  content: string;
  done: boolean;
}

// ── Session ──
export interface ChatSession {
  id: string;
  title: string;
  model: string;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
}

// ── User ──
export interface UserInfo {
  name: string;
  email?: string;
  avatarLetter: string;
  apiConfigs?: Record<string, string>;
}

// ── Settings ──
export interface AppSettings {
  theme: "dark" | "light" | "system";
  language: "zh-CN" | "en";
}
