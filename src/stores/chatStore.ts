import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ChatMessage, ToolCall, ToolProgress, StreamDelta, StreamDone, StreamError } from "../types/chat";

function genId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2);
}

function getActiveModel(): string {
  const agentId = localStorage.getItem("activeAgentId") || "hermes-builtin";
  const stored = localStorage.getItem(`agentModel_${agentId}`);
  return stored || "claude-sonnet-4-6";
}

interface ChatState {
  sessions: Array<{ id: string; title: string; model: string; agentId?: string; messageCount: number; createdAt: number; updatedAt: number }>;
  messages: ChatMessage[];
  sessionId: string | null;
  agentId: string;
  isLoading: boolean;
  error: string | null;
  toolCalls: ToolCall[];

  loadSessions: () => Promise<void>;
  setSessionId: (id: string) => void;
  setAgentId: (id: string) => void;
  clearMessages: () => void;
  addMessage: (role: "user" | "assistant" | "system", content: string) => ChatMessage;
  sendMessage: (content: string, model?: string) => Promise<void>;
  setupListeners: () => Promise<UnlistenFn>;
  abortChat: () => Promise<void>;

  appendDelta: (content: string, reasoningContent?: string) => void;
  upsertToolCall: (event: ToolProgress) => void;
}

let unlisteners: UnlistenFn[] | null = null;

async function ensureListeners() {
  if (unlisteners) return;
  const u1 = await listen<StreamDelta>("chat:delta", (evt) => {
    useChatStore.getState().appendDelta(
      evt.payload.content,
      evt.payload.reasoning_content ?? undefined,
    );
  });
  const u2 = await listen<StreamDone>("chat:done", async (evt) => {
    const state = useChatStore.getState();
    // Always use frontend sessionId — backend may return a different one
    const doneSessionId = state.sessionId;

    const now = Date.now();
    const finalizedToolCalls = state.toolCalls.map((tc) =>
      tc.status === "running" ? { ...tc, status: "completed" as const, completedAt: now } : tc
    );

    if (finalizedToolCalls.length > 0 && state.messages.length > 0) {
      const msgs = [...state.messages];
      const lastIdx = msgs.length - 1;
      if (msgs[lastIdx].role === "assistant") {
        msgs[lastIdx] = { ...msgs[lastIdx], isStreaming: false, toolCalls: finalizedToolCalls };
      }
      useChatStore.setState({ isLoading: false, error: null, toolCalls: [], messages: msgs });
    } else {
      const msgs = [...state.messages];
      const lastIdx = msgs.length - 1;
      if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
        msgs[lastIdx] = { ...msgs[lastIdx], isStreaming: false };
      }
      useChatStore.setState({ isLoading: false, error: null, toolCalls: [], messages: msgs });
    }

    const finalState = useChatStore.getState();
    const lastMsg = finalState.messages[finalState.messages.length - 1];
    if (lastMsg && lastMsg.role === "assistant" && lastMsg.content) {
      invoke("add_message", {
        sessionId: doneSessionId,
        role: "assistant",
        content: lastMsg.content,
      }).catch(() => {});

      const lastUser = [...finalState.messages].reverse().find(m => m.role === "user");

      const assistantCount = finalState.messages.filter(m => m.role === "assistant" && m.content).length;
      if (assistantCount === 1 && lastUser) {
        invoke("generate_title", {
          firstUserMsg: lastUser.content,
          firstAiMsg: lastMsg.content,
          model: getActiveModel(),
        }).then((title: unknown) => {
          if (typeof title === "string" && title && doneSessionId) {
            invoke("rename_session", { sessionId: doneSessionId, title }).catch(() => {});
          }
        }).catch(() => {});
      }
    }
  });
  const u3 = await listen<StreamError>("chat:error", (evt) => {
    useChatStore.setState({ isLoading: false, error: evt.payload.message, toolCalls: [] });
  });
  const u4 = await listen<ToolProgress>("chat:tool-progress", (evt) => {
    useChatStore.getState().upsertToolCall(evt.payload);
  });
  unlisteners = [u1, u2, u3, u4];
}

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  messages: [],
  sessionId: null,
  agentId: localStorage.getItem("activeAgentId") || "hermes-builtin",
  isLoading: false,
  error: null,
  toolCalls: [],

  loadSessions: async () => {
    try {
      const list = await invoke<Array<{ id: string; title: string; model: string; message_count: number; created_at: string; updated_at: string }>>("list_sessions");
      set({
        sessions: list.map((s) => ({
          id: s.id,
          title: s.title,
          model: s.model,
          messageCount: s.message_count,
          createdAt: new Date(s.created_at).getTime(),
          updatedAt: new Date(s.updated_at).getTime(),
        })),
      });
    } catch { /* ignore */ }
  },

  setSessionId: (id) => {
    set({ sessionId: id, messages: [], toolCalls: [], error: null });
    invoke("get_session", { sessionId: id }).then((detail: any) => {
      if (detail?.messages?.length) {
        set({ messages: detail.messages.map((m: any) => ({
          id: m.id || genId(),
          sessionId: id,
          role: m.role,
          content: m.content || "",
          timestamp: new Date(m.timestamp).getTime(),
        }))});
      }
    }).catch(() => {});
  },

  setAgentId: (id) => {
    set({ agentId: id });
    localStorage.setItem("activeAgentId", id);
  },

  clearMessages: () => set({ messages: [], toolCalls: [], error: null }),

  addMessage: (role, content) => {
    const msg: ChatMessage = {
      id: genId(),
      sessionId: get().sessionId || "default",
      role,
      content,
      timestamp: Date.now(),
    };
    set((s) => ({ messages: [...s.messages, msg] }));
    return msg;
  },

  appendDelta: (content, reasoningContent) => {
    const { messages, isLoading } = get();
    const lastMsg = messages[messages.length - 1];

    if (lastMsg && lastMsg.role === "assistant" && isLoading) {
      const updated: ChatMessage = { ...lastMsg, isStreaming: true };
      if (content) {
        updated.content = lastMsg.content + content;
        console.log(
          `[appendDelta] prev=${JSON.stringify(lastMsg.content.slice(-20))} + delta=${JSON.stringify(content)} → result=${JSON.stringify(updated.content.slice(-40))}`
        );
      }
      if (reasoningContent) {
        updated.thinking = (lastMsg.thinking || "") + reasoningContent;
      }
      set({ messages: [...messages.slice(0, -1), updated], error: null });
    } else if (content || reasoningContent) {
      const newMsg: ChatMessage = {
        id: genId(),
        sessionId: get().sessionId || "default",
        role: "assistant",
        content: content || "",
        timestamp: Date.now(),
        thinking: reasoningContent || undefined,
        isStreaming: true,
      };
      console.log(`[appendDelta] NEW msg with content=${JSON.stringify(content)}`);
      set({ messages: [...messages, newMsg], isLoading: true, error: null });
    }
  },

  upsertToolCall: (event) => {
    const { toolCalls, messages } = get();
    const callId = event.toolCallId || `${event.tool}_${Date.now()}`;
    const existingIdx = toolCalls.findIndex((tc) => tc.id === callId);

    if (event.status === "running" || (!event.status && !event.toolCallId)) {
      const newCall: ToolCall = {
        id: callId,
        tool: event.tool,
        label: event.label,
        emoji: event.emoji || "🔧",
        status: "running",
        startedAt: Date.now(),
      };
      const updated = existingIdx >= 0
        ? [...toolCalls.slice(0, existingIdx), newCall, ...toolCalls.slice(existingIdx + 1)]
        : [...toolCalls, newCall];

      const msgs = [...messages];
      let lastIdx = msgs.length - 1;
      if (lastIdx < 0 || msgs[lastIdx].role !== "assistant" || msgs[lastIdx].content) {
        msgs.push({
          id: genId(),
          sessionId: get().sessionId || "default",
          role: "assistant",
          content: "",
          timestamp: Date.now(),
          toolCalls: updated,
        });
        lastIdx = msgs.length - 1;
      } else {
        msgs[lastIdx] = { ...msgs[lastIdx], toolCalls: updated };
      }
      set({ toolCalls: updated, messages: msgs });
    } else if (event.status === "completed" && existingIdx >= 0) {
      const updated = toolCalls.map((tc) =>
        tc.id === callId ? { ...tc, status: "completed" as const, completedAt: Date.now() } : tc
      );
      const msgs = [...messages];
      const lastIdx = msgs.length - 1;
      if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
        msgs[lastIdx] = { ...msgs[lastIdx], toolCalls: updated };
      }
      set({ toolCalls: updated, messages: msgs });
    } else if (event.status === "error" && existingIdx >= 0) {
      const updated = toolCalls.map((tc) =>
        tc.id === callId ? { ...tc, status: "error" as const, completedAt: Date.now() } : tc
      );
      const msgs = [...messages];
      const lastIdx = msgs.length - 1;
      if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
        msgs[lastIdx] = { ...msgs[lastIdx], toolCalls: updated };
      }
      set({ toolCalls: updated, messages: msgs });
    }
  },

  sendMessage: async (content, model) => {
    // Ensure event listeners are ready before sending
    await ensureListeners();

    let { sessionId } = get();
    // Auto-generate session ID if user hasn't created one explicitly
    if (!sessionId) {
      sessionId = Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
      set({ sessionId });
    }

    // Add user message
    const userMsg: ChatMessage = {
      id: genId(),
      sessionId,
      role: "user",
      content,
      timestamp: Date.now(),
    };
    set((s) => ({
      messages: [...s.messages, userMsg],
      isLoading: true,
      error: null,
      toolCalls: [],
    }));

    // Build message list (before user was added), then add user
    const state = get();
    const msgs = state.messages.slice(0, -1).map((m) => ({
      role: m.role,
      content: m.content,
    }));
    msgs.push({ role: "user" as const, content });

    // Save session + user message, chained to avoid FK issues
    const { agentId } = get();
    const existingTitle = get().sessions.find(s => s.id === sessionId)?.title;
    invoke("upsert_session", {
      id: sessionId,
      title: existingTitle || content.slice(0, 30),
      model: getActiveModel(),
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      agentId: agentId || null,
    }).then(() => {
      // Save user message after session row exists
      invoke("add_message", {
        sessionId,
        role: "user",
        content,
      }).catch(() => {});
      // Refresh sessions in store
      get().loadSessions();
    }).catch((e) => {
      console.error("[chatStore] upsert_session failed:", e);
    });

    // Fire chat — result comes via events
    console.log("[chatStore] sendMessage sessionId=", sessionId);
    invoke("chat_completions", {
      messages: msgs,
      sessionId,
    }).catch((e: any) => {
      console.error("[chatStore] chat_completions failed:", e);
      set({ isLoading: false, error: String(e) });
    });
  },

  setupListeners: async () => {
    await ensureListeners();
    return async () => {
      for (const un of unlisteners || []) un();
      unlisteners = null;
    };
  },

  abortChat: async () => {
    await invoke("abort_chat").catch(() => {});
    set({ isLoading: false, toolCalls: [] });
  },
}));
