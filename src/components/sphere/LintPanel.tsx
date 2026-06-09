import { useCallback, useEffect, useRef } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import styles from "./LintPanel.module.css";

const LINT_LABELS: Record<string, string> = {
  orphan: "孤岛节点",
  dead_link: "死链接",
  low_confidence: "低置信度",
  stale: "过期实体",
  duplicate: "疑似重复",
};

const SEVERITY_COLORS: Record<string, string> = {
  high: "#ef4444",
  medium: "#f59e0b",
  low: "#94a3b8",
};

export function LintPanel() {
  const panelRef = useRef<HTMLDivElement>(null);
  const lintWarnings = useKnowledgeStore((s) => s.lintWarnings);
  const fetchLintWarnings = useKnowledgeStore((s) => s.fetchLintWarnings);
  const showLintPanel = useKnowledgeStore((s) => s.showLintPanel);
  const setShowLintPanel = useKnowledgeStore((s) => s.setShowLintPanel);
  const selectEntity = useKnowledgeStore((s) => s.selectEntity);
  const setFocusedNode = useKnowledgeStore((s) => s.setFocusedNode);

  useEffect(() => {
    if (showLintPanel) fetchLintWarnings();
  }, [showLintPanel, fetchLintWarnings]);

  const handleClose = useCallback(() => setShowLintPanel(false), [setShowLintPanel]);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        handleClose();
      }
    };
    const timer = setTimeout(() => document.addEventListener("mousedown", handler), 100);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("mousedown", handler);
    };
  }, [handleClose]);

  const handleClick = (w: typeof lintWarnings[0]) => {
    if (w.entity_id) {
      selectEntity(w.entity_id);
      setFocusedNode(w.entity_id);
    }
  };

  return (
    <div ref={panelRef} className={styles.panel}>
      <div className={styles.header}>
        <span className={styles.title}>Lint Warnings ({lintWarnings.length})</span>
        <button type="button" className={styles.closeBtn} onClick={handleClose} title="关闭" aria-label="关闭">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div className={styles.body}>
        {lintWarnings.length === 0 && (
          <div className={styles.empty}>暂无数据质量问题</div>
        )}
        {lintWarnings.map((w, i) => (
          <div
            key={`${w.warning_type}-${w.entity_id || i}`}
            className={styles.item}
            onClick={() => handleClick(w)}
            title={w.entity_id ? "点击聚焦实体" : undefined}
          >
            <span
              className={styles.severity}
              style={{ background: SEVERITY_COLORS[w.severity] || "#94a3b8" }}
            >
              {w.severity === "high" ? "H" : w.severity === "medium" ? "M" : "L"}
            </span>
            <span className={styles.label}>{LINT_LABELS[w.warning_type] || w.warning_type}</span>
            <span className={styles.message}>{w.message}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
