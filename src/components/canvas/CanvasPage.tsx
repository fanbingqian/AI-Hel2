import { useState, useCallback, useEffect, useRef } from "react";
import { Excalidraw } from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";
import { canvasOpen, canvasSave } from "../../services/api";
import styles from "./CanvasPage.module.css";

interface Props {
  filePath?: string | null;
}

export default function CanvasPage({ filePath }: Props = {}) {
  const [elements, setElements] = useState<any[]>([]);
  const [currentPath, setCurrentPath] = useState<string | null>(filePath ?? null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (filePath) {
      setCurrentPath(filePath);
      canvasOpen(filePath)
        .then((data: string) => {
          const parsed = JSON.parse(data);
          if (Array.isArray(parsed.elements)) {
            setElements(parsed.elements);
          }
        })
        .catch(console.error);
    }
  }, [filePath]);

  const handleChange = useCallback(
    (els: readonly any[], state: any, _files: any) => {
      setElements([...els]);
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        if (currentPath) {
          const data = {
            elements: els,
            app_state: {
              viewBackgroundColor: state.viewBackgroundColor,
              currentItemFontFamily: state.currentItemFontFamily,
            },
          };
          canvasSave(currentPath, JSON.stringify(data)).catch(console.error);
        }
      }, 1000);
    },
    [currentPath],
  );

  return (
    <div className={styles.canvas}>
      {currentPath ? (
        <div className={styles.toolbar}>
          <span className={styles.toolbarFilename}>{currentPath}</span>
          <span className={styles.toolbarSaved}>已保存</span>
        </div>
      ) : (
        <div className={styles.hint}>
          从知识文档页面选择一个 .excalidraw 文件打开，或在此新建画布
        </div>
      )}
      <div className={styles.excalContainer}>
        <Excalidraw
          key={currentPath || "empty"}
          initialData={{
            elements,
            appState: { viewBackgroundColor: "#1A1A1A" },
          }}
          onChange={handleChange}
          theme="dark"
          langCode="zh-CN"
        />
      </div>
    </div>
  );
}
