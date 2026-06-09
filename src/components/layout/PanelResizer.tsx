import { useCallback, useRef, useEffect } from "react";
import styles from "./PanelResizer.module.css";

interface Props {
  onResize: (delta: number) => void;
  direction?: "horizontal" | "vertical";
}

export function PanelResizer({ onResize, direction = "horizontal" }: Props) {
  const draggingRef = useRef(false);
  const startXRef = useRef(0);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      draggingRef.current = true;
      startXRef.current = e.clientX;
      document.body.style.cursor = direction === "horizontal" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
    },
    [direction],
  );

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!draggingRef.current) return;
      const delta = e.clientX - startXRef.current;
      startXRef.current = e.clientX;
      onResize(delta);
    };

    const handleMouseUp = () => {
      if (draggingRef.current) {
        draggingRef.current = false;
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      }
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [onResize]);

  return (
    <div
      className={`${styles.resizer} ${direction === "vertical" ? styles.vertical : ""}`}
      onMouseDown={handleMouseDown}
    />
  );
}
