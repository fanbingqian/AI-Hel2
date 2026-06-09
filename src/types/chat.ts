export interface ChatMessage {
  id: string;
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  isStreaming?: boolean;
  thinking?: string;
  toolCalls?: ToolCall[];
}

export interface ToolCall {
  id: string;
  tool: string;
  label: string;
  emoji: string;
  status: "running" | "completed" | "error";
  startedAt: number;
  completedAt?: number;
}

export interface ToolProgress {
  tool: string;
  label: string;
  emoji: string;
  toolCallId?: string;
  status?: string;
}

export interface ChatSession {
  id: string;
  title: string;
  model: string;
  agentId?: string;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
}

export interface StreamDelta {
  content: string;
  reasoning_content: string | null;
}

export interface StreamDone {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  session_id: string | null;
}

export interface StreamError {
  message: string;
  retryable: boolean;
}
