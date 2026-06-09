import { useState, useCallback, useEffect, useRef } from "react";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import styles from "./FloatingMenu.module.css";

interface Props {
  onOpenSettings: () => void;
}

export function FloatingMenu({ onOpenSettings }: Props) {
  const [expanded, setExpanded] = useState(false);
  const graphViewMode = useKnowledgeStore((s) => s.graphViewMode);
  const setGraphViewMode = useKnowledgeStore((s) => s.setGraphViewMode);
  const showLintPanel = useKnowledgeStore((s) => s.showLintPanel);
  const setShowLintPanel = useKnowledgeStore((s) => s.setShowLintPanel);
  const showEntityList = useKnowledgeStore((s) => s.showEntityList);
  const setShowEntityList = useKnowledgeStore((s) => s.setShowEntityList);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!expanded) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setExpanded(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [expanded]);

  const handleSelect = useCallback((mode: "2d" | "3d") => {
    setGraphViewMode(mode);
    setExpanded(false);
  }, [setGraphViewMode]);

  const handleLint = useCallback(() => {
    setShowLintPanel(!showLintPanel);
    setExpanded(false);
  }, [showLintPanel, setShowLintPanel]);

  const handleEntityList = useCallback(() => {
    setShowEntityList(!showEntityList);
    setExpanded(false);
  }, [showEntityList, setShowEntityList]);

  const handleSettings = useCallback(() => {
    onOpenSettings();
    setExpanded(false);
  }, [onOpenSettings]);

  return (
    <div ref={menuRef} className={styles.wrap}>
      {expanded && <div className={styles.backdrop} />}
      {expanded && (
        <div className={styles.menu}>
          <button
            type="button"
            className={`${styles.menuItem} ${graphViewMode === "2d" ? styles.menuItemActive : ""}`}
            onClick={() => handleSelect("2d")}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <rect x="3" y="3" width="18" height="18" rx="2"/>
              <line x1="3" y1="9" x2="21" y2="9"/>
              <line x1="9" y1="3" x2="9" y2="21"/>
            </svg>
            <span>2D 图谱</span>
          </button>
          <button
            type="button"
            className={`${styles.menuItem} ${graphViewMode === "3d" ? styles.menuItemActive : ""}`}
            onClick={() => handleSelect("3d")}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
              <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
              <line x1="12" y1="22.08" x2="12" y2="12"/>
            </svg>
            <span>3D 图谱</span>
          </button>
          {graphViewMode === "2d" && (
            <>
              <button
                type="button"
                className={`${styles.menuItem} ${showLintPanel ? styles.menuItemActive : ""}`}
                onClick={handleLint}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
                  <line x1="12" y1="9" x2="12" y2="13"/>
                  <line x1="12" y1="17" x2="12.01" y2="17"/>
                </svg>
                <span>数据质量</span>
              </button>
              <button
                type="button"
                className={`${styles.menuItem} ${showEntityList ? styles.menuItemActive : ""}`}
                onClick={handleEntityList}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                  <line x1="3" y1="6" x2="21" y2="6"/>
                  <line x1="3" y1="12" x2="21" y2="12"/>
                  <line x1="3" y1="18" x2="21" y2="18"/>
                </svg>
                <span>实体列表</span>
              </button>
            </>
          )}
          <button
            type="button"
            className={styles.menuItem}
            onClick={handleSettings}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <circle cx="12" cy="12" r="3"/>
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
            </svg>
            <span>图谱配置</span>
          </button>
        </div>
      )}
      <button
        type="button"
        className={`${styles.toggle} ${expanded ? styles.toggleActive : ""}`}
        onClick={() => setExpanded(!expanded)}
        title="图谱选项"
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
          <circle cx="12" cy="12" r="10"/>
          <circle cx="12" cy="12" r="3"/>
          <line x1="12" y1="2" x2="12" y2="4"/>
          <line x1="12" y1="20" x2="12" y2="22"/>
          <line x1="2" y1="12" x2="4" y2="12"/>
          <line x1="20" y1="12" x2="22" y2="12"/>
        </svg>
      </button>
    </div>
  );
}
