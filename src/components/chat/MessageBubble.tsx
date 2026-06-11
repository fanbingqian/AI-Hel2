import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { useState, useCallback } from "react";
import type { ChatMessage } from "../../types/chat";
import { ThinkingSection } from "./ThinkingSection";
import { ToolCallTimeline } from "./ToolCallTimeline";
import { useChatStore } from "../../stores/chatStore";
import { saveChatToKnowledge } from "../../services/api";
import { open } from "@tauri-apps/plugin-shell";
import styles from "./MessageBubble.module.css";
import "highlight.js/styles/github-dark-dimmed.css";

interface Props {
  message: ChatMessage;
}

function MarkdownRenderer({ content }: { content: string }) {
  const handleLinkClick = useCallback((e: React.MouseEvent) => {
    const href = (e.currentTarget as HTMLAnchorElement).getAttribute("href");
    if (href && /^https?:\/\//i.test(href)) {
      e.preventDefault();
      // Use Tauri shell open → system browser, NOT the app webview
      open(href).catch(() => window.open(href, "_blank"));
      // System notification as a "come back" reminder
      try {
        new Notification("AI-Hel2", {
          body: "操作完成后请返回 AI-Hel2，点击「已完成操作」继续对话",
          requireInteraction: true,
        });
      } catch {}
    }
  }, []);

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeHighlight]}
      components={{
        a: ({ href, children, ...props }: any) => (
          <a href={href} onClick={handleLinkClick} {...props}>
            {children}
          </a>
        ),
      }}
    >
      {content}
    </ReactMarkdown>
  );
}

export function MessageBubble({ message }: Props) {
  const isStreaming = message.isStreaming === true;
  const role = message.role;
  const isLoading = useChatStore((s) => s.isLoading);
  const messages = useChatStore((s) => s.messages);
  const isLastMessage = messages.length > 0 && messages[messages.length - 1].id === message.id;
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const handleSaveToKnowledge = useCallback(async () => {
    if (saving || saved || !message.content) return;
    setSaving(true);
    try {
      // Extract a meaningful title from the content:
      // 1. First Markdown heading (# Title)
      // 2. First non-empty meaningful line (at least 8 chars)
      // 3. Fallback to first 40 chars
      const lines = message.content.split("\n");
      let title = "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.startsWith("# ") && trimmed.length > 3) {
          title = trimmed.replace(/^#+\s*/, "").slice(0, 60);
          break;
        }
      }
      if (!title) {
        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed.length >= 8 && !trimmed.startsWith("```") && !trimmed.startsWith("- ") && !trimmed.match(/^\d+\./)) {
            title = trimmed.slice(0, 60);
            break;
          }
        }
      }
      if (!title) {
        title = message.content.replace(/\n/g, " ").slice(0, 40);
      }
      const result: any = await saveChatToKnowledge(message.content, title);
      setSaved(true);
      // Show extraction count to user via alert or we can skip
      console.log(`Saved: ${title}, entities: ${result?.new_count || 0}`);
    } catch (e) {
      console.error("Save to knowledge failed:", e);
    } finally {
      setSaving(false);
    }
  }, [message.content, saving, saved]);

  return (
    <div className={`${styles.wrapper} ${styles[role]}`}>
      {role !== "system" && (
        <div className={`${styles.meta} ${role === "user" ? styles.metaRight : styles.metaLeft}`}>
          <span className={styles.role}>{role === "user" ? "你" : "AI"}</span>
          <span className={styles.time}>
            {new Date(message.timestamp).toLocaleTimeString()}
          </span>
        </div>
      )}

      {role === "assistant" && message.thinking && (
        <ThinkingSection
          content={message.thinking}
          isStreaming={isLoading && isLastMessage}
        />
      )}

      {role === "assistant" && message.toolCalls && message.toolCalls.length > 0 && (
        <ToolCallTimeline toolCalls={message.toolCalls} />
      )}

      <div
        className={`${styles.bubble} ${styles[role]} ${isStreaming ? styles.streaming : ""}`}
      >
        {role === "user" ? (
          <p>{message.content}</p>
        ) : (
          message.content ? (
            <MarkdownRenderer content={message.content} />
          ) : (
            isStreaming ? "" : " "
          )
        )}
      </div>

      {role === "assistant" && !isStreaming && message.content && (
        <div className={styles.bubbleActions}>
          <button
            type="button"
            className={`${styles.saveToKnowledgeBtn} ${saved ? styles.saved : ""}`}
            onClick={handleSaveToKnowledge}
            disabled={saving || saved}
            title={saved ? "已存入知识库" : "存入知识库"}
          >
            {saved ? "已存入" : saving ? "存入中…" : "存入知识库"}
          </button>
        </div>
      )}
    </div>
  );
}
