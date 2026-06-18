import { useEffect, useRef, useState, useCallback } from "react";
import { useChatStore } from "../../stores/chatStore";
import { useUIStore } from "../../stores/uiStore";
import { useSettingsStore } from "../../stores/settingsStore";
import { useVoiceStore } from "../../stores/voiceStore";
import { MessageBubble } from "./MessageBubble";
import { VoiceInput } from "./VoiceInput";
import { useTTS } from "../../hooks/useTTS";
import { open } from "@tauri-apps/plugin-dialog";
import { readTextFile, captureScreen } from "../../services/api";
import styles from "./ChatPanel.module.css";

const SPEAKERS: { id: number; name: string; gender: string }[] = [
  { id: 0, name: "苏映雪", gender: "女" },
  { id: 1, name: "顾年", gender: "男" },
  { id: 2, name: "傅诗雨", gender: "女" },
  { id: 3, name: "病娇", gender: "女" },
  { id: 4, name: "霸总", gender: "男" },
];

export function ChatPanel() {
  const chatPanelWidth = useUIStore((s) => s.chatPanelWidth);
  const panelCollapsed = useUIStore((s) => s.panelCollapsed);
  const messages = useChatStore((s) => s.messages);
  const sessionId = useChatStore((s) => s.sessionId);
  const isLoading = useChatStore((s) => s.isLoading);
  const error = useChatStore((s) => s.error);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const addMessage = useChatStore((s) => s.addMessage);
  const abortChat = useChatStore((s) => s.abortChat);
  const setupListeners = useChatStore((s) => s.setupListeners);
  const toggleSessionList = useUIStore((s) => s.toggleSessionList);

  const ttsEnabled = useSettingsStore((s) => s.ttsEnabled);
  const ttsSpeaker = useSettingsStore((s) => s.ttsSpeaker);
  const setTtsEnabled = useSettingsStore((s) => s.setTtsEnabled);
  const setTtsSpeaker = useSettingsStore((s) => s.setTtsSpeaker);

  const inputMode = useVoiceStore((s) => s.inputMode);
  const voiceStatus = useVoiceStore((s) => s.status);
  const toggleInputMode = useVoiceStore((s) => s.toggleInputMode);

  const [input, setInput] = useState("");
  const [speakerOpen, setSpeakerOpen] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const spokenMsgIds = useRef<Set<string>>(new Set());
  const { speakSegments, stop: stopTTS } = useTTS();

  // Auto-speak assistant responses via TTS
  useEffect(() => {
    if (!ttsEnabled) return;
    const lastMsg = messages[messages.length - 1];
    if (
      lastMsg &&
      lastMsg.role === "assistant" &&
      lastMsg.content &&
      lastMsg.isStreaming === false &&
      !spokenMsgIds.current.has(lastMsg.id)
    ) {
      spokenMsgIds.current.add(lastMsg.id);
      speakSegments(lastMsg.content, ttsSpeaker);
    }
  }, [messages, ttsEnabled, ttsSpeaker, speakSegments]);

  const handleToggleTTS = useCallback(() => {
    if (ttsEnabled) stopTTS();
    setTtsEnabled(!ttsEnabled);
  }, [ttsEnabled, stopTTS, setTtsEnabled]);

  useEffect(() => {
    const cleanup = setupListeners();
    return () => { cleanup.then((fn: () => void) => fn()); };
  }, [setupListeners]);

  // First-launch greeting — fires once when session is fresh and has no messages
  const greeted = useRef(false);
  useEffect(() => {
    if (greeted.current) return;
    if (sessionId && messages.length === 0 && !isLoading) {
      greeted.current = true;
      const hour = new Date().getHours();
      const timeGreeting = hour < 6 ? "夜深了，注意休息" : hour < 12 ? "早上好" : hour < 14 ? "中午好" : hour < 18 ? "下午好" : "晚上好";
      // Auto-send as if the agent greeted the user
      addMessage("assistant", `${timeGreeting}，欢迎使用 AI-Hel2 伴随式智能体`);
    }
  }, [sessionId, messages.length, isLoading]);

  // Scroll to bottom: instant during streaming (content height changing
  // rapidly), smooth only when a new message appears.  Prevents the jitter
  // caused by overlapping smooth-scroll animations on every delta.
  const prevCountRef = useRef(messages.length);
  useEffect(() => {
    const prevCount = prevCountRef.current;
    prevCountRef.current = messages.length;
    const isStreaming =
      isLoading && messages.length > 0 &&
      messages[messages.length - 1]?.isStreaming === true;
    const isNew = messages.length > prevCount;
    const behavior: ScrollBehavior = isStreaming
      ? "instant"
      : isNew
        ? "smooth"
        : "auto";
    messagesEndRef.current?.scrollIntoView({ behavior });
  }, [messages, isLoading]);

  // Auto-grow textarea on input (cap at 5 lines ≈ 110px, scroll beyond)
  const handleTextareaInput = useCallback((e: React.FormEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 110) + "px";
  }, []);

  // Ctrl+Space toggles input mode
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.code === "Space") {
        e.preventDefault();
        toggleInputMode();
      }
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [toggleInputMode]);

  // Right Alt PTT (push-to-talk): hold to record, release to transcribe
  useEffect(() => {
    const altRightDownRef = { current: false };

    const handleKeyDown = (e: KeyboardEvent) => {
      // Only Right Alt (not left), ignore key-repeat, ignore if AltGr composing
      if (e.code !== "AltRight" || e.repeat || e.getModifierState("AltGraph")) return;
      e.preventDefault();
      if (altRightDownRef.current) return;
      altRightDownRef.current = true;
      const { status } = useVoiceStore.getState();
      if (status === "idle") {
        useVoiceStore.getState().startRecording("chat");
      }
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      if (e.code !== "AltRight") return;
      e.preventDefault();
      altRightDownRef.current = false;
      const { status } = useVoiceStore.getState();
      if (status === "listening") {
        useVoiceStore.getState().stopRecording();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("keyup", handleKeyUp);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("keyup", handleKeyUp);
    };
  }, []);

  const handleSend = () => {
    const text = input.trim();
    if (!text || isLoading) return;
    setInput("");
    sendMessage(text);
  };

  const handleFileUpload = async () => {
    try {
      const selected = await open({ multiple: false });
      if (selected) {
        const path = typeof selected === "string" ? selected : (selected as { path: string }).path;
        if (path) {
          const content = await readTextFile(path);
          setInput((prev) => prev + content);
        }
      }
    } catch { /* user cancelled */ }
  };

  const handleScreenshot = async () => {
    try {
      const base64 = await captureScreen();
      if (base64) {
        setInput((prev) => prev + `![screenshot](${base64})`);
      }
    } catch (e) {
      console.error("Screenshot failed:", e);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const isVoiceMode = inputMode === "voice";
  const isVoiceActive = voiceStatus !== "idle";

  return (
    <div className={styles.panel} style={panelCollapsed ? { width: "100%", minWidth: 0 } : { width: chatPanelWidth, minWidth: chatPanelWidth }}>
      <div className={styles.messages}>
        {error && (
          <div className={styles.errorBanner}>
            <span className={styles.errorIcon}>!</span>
            <span className={styles.errorText}>{error}</span>
            <button type="button" className={styles.errorDismiss} onClick={() => { useChatStore.setState({ error: null }); }}>x</button>
          </div>
        )}
        {messages.length === 0 && !error && (
          <div className={styles.emptyState}>
            <div className={styles.emptyText}>说 "Hi Hel" 唤醒我</div>
            <div className={styles.emptySubText}>或直接在下方输入文字</div>
          </div>
        )}
        {messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      <div className={`${styles.inputBar} ${isVoiceActive ? styles.inputBarListening : ""}`}>
        {isVoiceMode || isVoiceActive ? (
          <VoiceInput />
        ) : (
          <textarea
            ref={textareaRef}
            id="chat-input"
            name="message"
            className={styles.textInput}
            placeholder="输入消息... (Enter 发送, 右Alt 按住说话)"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onInput={handleTextareaInput}
            rows={1}
            onKeyDown={handleKeyDown}
            aria-label="输入消息"
          />
        )}
        <div className={styles.inputFoot}>
          <div className={styles.inputFootLeft}>
            <button type="button" className={`${styles.inputTool} ${ttsEnabled ? styles.inputToolActive : ""}`} onClick={handleToggleTTS} title={ttsEnabled ? "关闭语音播报" : "开启语音播报"}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M11 5L6 9H2v6h4l5 4V5z"/>
                {ttsEnabled && (
                  <>
                    <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                    <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                  </>
                )}
                {!ttsEnabled && <line x1="23" y1="1" x2="1" y2="23" />}
              </svg>
            </button>
            {ttsEnabled && (
              <div className={styles.speakerWrap}>
                <button
                  type="button"
                  className={styles.speakerBtn}
                  onClick={() => setSpeakerOpen(!speakerOpen)}
                  title="选择音色"
                >
                  {SPEAKERS.find((s) => s.id === ttsSpeaker)?.name || "苏映雪"}
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M6 9l6 6 6-6" />
                  </svg>
                </button>
                {speakerOpen && (
                  <div className={styles.speakerDropdown}>
                    {SPEAKERS.map((s) => (
                      <button
                        key={s.id}
                        type="button"
                        className={`${styles.speakerItem} ${s.id === ttsSpeaker ? styles.speakerItemActive : ""}`}
                        onClick={() => { setTtsSpeaker(s.id); setSpeakerOpen(false); }}
                      >
                        {s.name} ({s.gender})
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
            {/* Input mode toggle */}
            <button
              type="button"
              className={`${styles.inputTool} ${isVoiceMode ? styles.inputToolActive : ""}`}
              onClick={toggleInputMode}
              title={isVoiceMode ? "切换到文字输入 (Ctrl+Space)" : "切换到语音输入 (Ctrl+Space，右Alt 按住说话)"}
            >
              {isVoiceMode ? (
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <rect x="2" y="4" width="20" height="16" rx="2"/>
                  <line x1="6" y1="9" x2="18" y2="9"/>
                  <line x1="6" y1="12" x2="18" y2="12"/>
                  <line x1="6" y1="15" x2="12" y2="15"/>
                </svg>
              ) : (
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <rect x="9" y="1" width="6" height="14" rx="3"/>
                  <path d="M5 10a7 7 0 0 0 14 0"/>
                </svg>
              )}
            </button>
            <button type="button" className={styles.inputTool} onClick={toggleSessionList} title="会话列表">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <rect x="3" y="3" width="18" height="18" rx="2"/>
                <line x1="3" y1="9" x2="21" y2="9"/>
                <line x1="3" y1="15" x2="21" y2="15"/>
                <line x1="9" y1="3" x2="9" y2="21"/>
              </svg>
            </button>
            <button type="button" className={styles.inputTool} onClick={handleFileUpload} title="上传文件">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>
              </svg>
            </button>
            <button type="button" className={styles.inputTool} onClick={handleScreenshot} title="截图">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <rect x="3" y="3" width="18" height="18" rx="2"/>
                <circle cx="8.5" cy="8.5" r="1.5"/>
                <path d="M21 15l-5-5L5 21"/>
              </svg>
            </button>
          </div>
          {(isVoiceMode || isVoiceActive) ? (
            isVoiceActive && (
              <button className={styles.sendBtn} onClick={() => useVoiceStore.getState().cancelRecording()} style={{ background: "#e74c3c" }}>
                取消
              </button>
            )
          ) : (
            <div className={styles.sendGroup}>
              {messages.length > 0 && !isLoading && (
                <button
                  type="button"
                  className={styles.quickContinueBtn}
                  onClick={() => sendMessage("已完成你上一步让我做的操作，请继续。")}
                  title="通知 Agent 你已完成外部操作（如浏览器登录）"
                >
                  已完成操作
                </button>
              )}
              {isLoading ? (
                <button type="button" className={styles.sendBtn} onClick={abortChat}>停止</button>
              ) : (
                <button type="button" className={styles.sendBtn} onClick={handleSend} disabled={!input.trim()}>发送</button>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
