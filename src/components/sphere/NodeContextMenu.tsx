import { useEffect, useRef } from "react";
import styles from "./NodeContextMenu.module.css";

interface Props {
  x: number;
  y: number;
  entityName: string;
  onDetail: () => void;
  onReference: () => void;
  onFocus: () => void;
  onClose: () => void;
}

export function NodeContextMenu({ x, y, entityName, onDetail, onReference, onFocus, onClose }: Props) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const keyHandler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", handler);
    document.addEventListener("keydown", keyHandler);
    return () => {
      document.removeEventListener("mousedown", handler);
      document.removeEventListener("keydown", keyHandler);
    };
  }, [onClose]);

  // Adjust position to stay within viewport
  const adjustedX = Math.min(x, window.innerWidth - 180);
  const adjustedY = Math.min(y, window.innerHeight - 140);

  return (
    <div
      ref={menuRef}
      className={styles.menu}
      style={{ left: adjustedX, top: adjustedY }}
    >
      <div className={styles.header}>{entityName}</div>
      <button className={styles.item} onClick={onDetail}>
        <span className={styles.icon}>◎</span>
        查看详情
      </button>
      <button className={styles.item} onClick={onReference}>
        <span className={styles.icon}>↗</span>
        引用到对话
      </button>
      <button className={styles.item} onClick={onFocus}>
        <span className={styles.icon}>◎</span>
        聚焦
      </button>
    </div>
  );
}
