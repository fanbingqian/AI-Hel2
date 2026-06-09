import { useState, useEffect, useRef } from "react";
import { Check, Loader2, Search, Globe, FileText, Terminal, Wrench, ChevronRight, ChevronDown } from "lucide-react";
import type { ToolCall } from "../../types/chat";
import styles from "./ToolCallTimeline.module.css";

interface Props {
  toolCalls: ToolCall[];
}

const TOOL_ICONS: Record<string, React.ComponentType<any>> = {
  web_search: Search,
  web_extract: Globe,
  web_fetch: Globe,
  read_file: FileText,
  terminal: Terminal,
  execute_code: Terminal,
};

function getIcon(tool: string) {
  const Cmp = TOOL_ICONS[tool] || Wrench;
  return <Cmp size={13} className={styles.rowIconSvg} />;
}

function humanToolName(tool: string): string {
  switch (tool) {
    case "web_search":       return "搜索网页";
    case "web_extract":
    case "web_fetch":        return "读取网页";
    case "read_file":        return "读取文件";
    case "write_file":
    case "patch":            return "写入文件";
    case "terminal":         return "执行命令";
    case "execute_code":     return "执行代码";
    case "search_files":     return "搜索文件";
    case "delegate_task":    return "委派任务";
    case "memory":           return "记忆操作";
    default:                 return tool;
  }
}

function formatDuration(ms: number): string {
  const sec = ms / 1000;
  if (sec < 1) return `${Math.round(sec * 10) / 10}s`;
  if (sec < 60) return `${Math.round(sec * 10) / 10}s`;
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  return `${m}m${s}s`;
}

function StatusIcon({ status }: { status: ToolCall["status"] }) {
  switch (status) {
    case "running":
      return <Loader2 size={12} className={styles.spinner} />;
    case "completed":
      return <Check size={13} className={styles.check} />;
    case "error":
      return <span className={styles.errorMark}>!</span>;
  }
}

function buildSummary(calls: ToolCall[]): string {
  const completed = calls.filter((c) => c.status === "completed").length;
  const running = calls.filter((c) => c.status === "running").length;
  const total = calls.length;

  if (running > 0) {
    const runningCall = calls.find((c) => c.status === "running");
    const name = runningCall ? humanToolName(runningCall.tool) : "工具";
    if (completed > 0) {
      return `✓ ${completed}/${total} · ${name}...`;
    }
    if (runningCall && runningCall.label && runningCall.label !== runningCall.tool) {
      const label = runningCall.label.length > 20 ? runningCall.label.slice(0, 20) + "..." : runningCall.label;
      return `${name} · "${label}"...`;
    }
    return `${name}...`;
  }

  const groups = new Map<string, number>();
  for (const c of calls) {
    const name = humanToolName(c.tool);
    groups.set(name, (groups.get(name) || 0) + 1);
  }
  const parts = [...groups.entries()].map(([name, count]) => (count > 1 ? `${name} ×${count}` : name));

  const totalMs = calls.reduce((sum, c) => {
    if (c.completedAt) return sum + (c.completedAt - c.startedAt);
    return sum;
  }, 0);

  return `✓ ${parts.join(" · ")} · ${formatDuration(totalMs)}`;
}

export function ToolCallTimeline({ toolCalls }: Props) {
  const [userOverride, setUserOverride] = useState<boolean | null>(null);
  const autoCollapseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hadRunningRef = useRef(false);

  const hasRunning = toolCalls.some((c) => c.status === "running");
  const allDone = toolCalls.length > 0 && toolCalls.every((c) => c.status !== "running");

  if (hasRunning) hadRunningRef.current = true;

  useEffect(() => {
    if (allDone && hadRunningRef.current && userOverride === null) {
      autoCollapseTimer.current = setTimeout(() => {
        setUserOverride(false);
      }, 1500);
    }
    return () => {
      if (autoCollapseTimer.current) clearTimeout(autoCollapseTimer.current);
    };
  }, [allDone, userOverride]);

  useEffect(() => {
    if (hasRunning) {
      hadRunningRef.current = true;
      if (userOverride === null) setUserOverride(true);
    }
  }, [hasRunning, userOverride]);

  if (toolCalls.length === 0) return null;

  const expanded = userOverride ?? hasRunning;
  const Chevron = expanded ? ChevronDown : ChevronRight;

  return (
    <div className={`${styles.panel} ${hasRunning ? styles.panelLive : ""}`}>
      <button
        type="button"
        className={styles.header}
        onClick={() => setUserOverride(!expanded)}
        aria-expanded={expanded ? "true" : "false"}
      >
        <Chevron className={styles.chevron} />
        {hasRunning ? (
          <Loader2 size={12} className={styles.headerSpinner} />
        ) : (
          <Check size={13} className={styles.headerCheck} />
        )}
        <span className={styles.headerSummary}>{buildSummary(toolCalls).replace(/^✓ /, "")}</span>
        {hasRunning && <span className={styles.dot} />}
      </button>

      {expanded && (
        <div className={styles.body}>
          {toolCalls.map((tc) => {
            const duration = tc.completedAt ? tc.completedAt - tc.startedAt : null;
            const query = tc.tool === "web_search" && tc.label && tc.label !== tc.tool ? tc.label : null;
            const detail = tc.tool !== "web_search" && tc.label && tc.label !== tc.tool ? tc.label : null;
            return (
              <div
                key={tc.id}
                className={`${styles.row} ${tc.status === "running" ? styles.rowLive : ""}`}
              >
                <span className={styles.rowStatus}>
                  <StatusIcon status={tc.status} />
                </span>
                <span className={styles.rowIcon}>{getIcon(tc.tool)}</span>
                <span className={styles.rowName}>{humanToolName(tc.tool)}</span>
                {query && <span className={styles.rowQuery}>"{query}"</span>}
                {detail && <span className={styles.rowDetail}>{detail}</span>}
                {duration && tc.status === "completed" && (
                  <span className={styles.rowTime}>{formatDuration(duration)}</span>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
