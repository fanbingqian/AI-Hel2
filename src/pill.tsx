import React from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";

interface AgentStep {
  tool: string;
  label: string;
  status: "running" | "done" | "error";
  emoji?: string;
}

interface AgentStatus {
  working: boolean;
  message?: string;
  elapsed?: number;
  steps?: AgentStep[];
}

const PILL_W = 290;
const PILL_H_IDLE = 52;
const PILL_H_WORKING = 80;

const css = {
  pill: (expanded: boolean) => ({
    display: "flex", flexDirection: "column" as const,
    width: PILL_W, height: expanded ? PILL_H_WORKING : PILL_H_IDLE,
    borderRadius: 14, overflow: "hidden",
    background: "rgba(26, 22, 38, 0.6)",
    backdropFilter: "blur(22px) saturate(160%)",
    WebkitBackdropFilter: "blur(22px) saturate(160%)",
    border: "1.5px solid rgba(7, 193, 96, 0.45)",
    boxShadow: "0 4px 14px rgba(0,0,0,0.22), 0 0 20px rgba(7,193,96,0.22)",
    cursor: "pointer",
    userSelect: "none" as const,
    transition: "height 0.3s ease, border-color 0.3s",
  }),
  mainRow: {
    display: "flex", alignItems: "center", gap: 9,
    height: PILL_H_IDLE, padding: "0 14px 0 11px",
    flexShrink: 0,
  } as React.CSSProperties,
  label: {
    flex: 1, color: "rgba(255,255,255,0.92)", fontSize: 12, fontWeight: 600,
    letterSpacing: "0.6px", textShadow: "0 1px 3px rgba(0,0,0,0.5)",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
  } as React.CSSProperties,
  dot: (active: boolean) => ({
    width: 7, height: 7, borderRadius: "50%", flexShrink: 0,
    background: active ? "#f59e0b" : "#07c160",
    boxShadow: active ? "0 0 8px #f59e0b" : "0 0 8px #07c160",
    transition: "background 0.3s, box-shadow 0.3s",
  }),
  detail: {
    padding: "4px 14px 12px", color: "rgba(255,255,255,0.6)",
    fontSize: 10, lineHeight: 1.7, overflow: "hidden",
  } as React.CSSProperties,
  detailMsg: {
    marginBottom: 2, color: "rgba(255,255,255,0.7)", fontSize: 11,
  } as React.CSSProperties,
  stepRow: {
    display: "flex", alignItems: "center", gap: 6, fontSize: 10,
  } as React.CSSProperties,
  stepIcon: (status: string) => ({
    width: 12, textAlign: "center" as const, flexShrink: 0,
    color: status === "done" ? "#07c160" : status === "error" ? "#ef4444" : "#f59e0b",
  }),
};

export function Pill() {
  const [status, setStatus] = React.useState<AgentStatus>({ working: false });
  const [hovered, setHovered] = React.useState(false);
  const expanded = status.working || hovered;

  // Transparent, borderless window — capsule IS the window
  React.useEffect(() => {
    document.documentElement.style.cssText = "background:transparent!important;margin:0;padding:0;overflow:hidden;";
    document.body.style.cssText = "background:transparent!important;margin:0;padding:0;overflow:hidden;width:100%;height:100%;";
    const root = document.getElementById("root");
    if (root) { root.style.cssText = "background:transparent!important;width:100%;height:100%;display:flex;align-items:flex-start;justify-content:center;"; }
  }, []);

  // Listen for agent status events from main window
  React.useEffect(() => {
    const p = listen<AgentStatus>("agent:status", (e) => setStatus(e.payload));
    return () => { p.then((fn) => fn()); };
  }, []);

  // Double-click to toggle main window
  const lastClickRef = React.useRef(0);
  const handleClick = () => {
    const now = Date.now();
    if (now - lastClickRef.current < 400) {
      lastClickRef.current = 0;
      invoke("toggle_main_window");
    } else {
      lastClickRef.current = now;
    }
  };

  // Auto-resize pill window height when working state changes
  React.useEffect(() => {
    const win = getCurrentWindow();
    const h = status.working ? PILL_H_WORKING : PILL_H_IDLE;
    win.setSize(new LogicalSize(PILL_W, h));
  }, [status.working]);

  const working = status.working;
  const label = working
    ? `${status.message || "工作中…"}${status.elapsed ? ` ${status.elapsed}s` : ""}`
    : "AI-Hel2 · 在线";

  return (
    <div
      style={css.pill(expanded)}
      onClick={handleClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div style={css.mainRow}>
        {/* 🤖 Robot icon */}
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#07c160" strokeWidth="1.5">
          <rect x="3" y="5" width="18" height="14" rx="3" />
          <circle cx="8.5" cy="11" r="1.5" fill="#07c160" />
          <circle cx="15.5" cy="11" r="1.5" fill="#07c160" />
          <line x1="12" y1="14" x2="12" y2="17" stroke="#07c160" strokeWidth="1.2" />
          <line x1="9" y1="17" x2="15" y2="17" stroke="#07c160" strokeWidth="1.2" />
          <line x1="10" y1="3" x2="14" y2="3" stroke="#07c160" strokeWidth="1.2" />
        </svg>
        <span style={css.label}>{label}</span>
        <span style={css.dot(working)} />
      </div>
      {expanded && (
        <div style={css.detail}>
          {working ? (
            <div style={css.detailMsg}>工作中，双击打开对话窗口…</div>
          ) : (
            <div style={css.detailMsg}>双击打开对话窗口</div>
          )}
        </div>
      )}
    </div>
  );
}
