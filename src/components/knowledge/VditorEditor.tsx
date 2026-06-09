import { useEffect, useRef, useState, useCallback } from "react";
import Vditor from "vditor";
import "vditor/dist/index.css";
import * as api from "../../services/api";
import styles from "./VditorEditor.module.css";

interface Props {
  filePath: string | null;
}

export function VditorEditor({ filePath }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const vditorRef = useRef<Vditor | null>(null);
  const vditorReadyRef = useRef(false);
  const pendingRef = useRef<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const contentRef = useRef("");

  const safeSetValue = useCallback((value: string) => {
    if (vditorReadyRef.current && vditorRef.current) {
      vditorRef.current.setValue(value);
    } else {
      pendingRef.current = value;
    }
  }, []);

  useEffect(() => {
    if (!filePath) {
      contentRef.current = "";
      setError(null);
      setLoading(false);
      setDirty(false);
      safeSetValue("");
      return;
    }
    setLoading(true);
    setError(null);
    api.readWikiFile(filePath)
      .then((data) => {
        contentRef.current = data;
        setLoading(false);
        setDirty(false);
        safeSetValue(data);
      })
      .catch((e) => {
        setError(`读取失败: ${e}`);
        contentRef.current = "";
        setLoading(false);
      });
  }, [filePath, safeSetValue]);

  useEffect(() => {
    if (!containerRef.current) return;
    let disposed = false;
    const vditor = new Vditor(containerRef.current, {
      cache: { id: "vditor-main" },
      mode: "ir",
      theme: "dark",
      placeholder: "开始编辑...",
      outline: { enable: true, position: "right" },
      counter: { enable: true },
      preview: {
        hljs: { style: "github-dark-dimmed" },
        theme: { current: "dark" },
      },
      toolbar: [
        "undo", "redo", "|",
        "headings", "bold", "italic", "strike", "|",
        "quote", "code", "inline-code", "|",
        "list", "ordered-list", "check", "|",
        "link", "table", "|",
        "outline", "fullscreen",
      ],
      toolbarConfig: { hide: false },
      input(value) {
        contentRef.current = value;
        setDirty(true);
      },
      after() {
        if (disposed) return;
        vditorReadyRef.current = true;
        const pending = pendingRef.current;
        if (pending !== null) {
          vditor.setValue(pending);
          pendingRef.current = null;
        }
      },
    });
    vditorRef.current = vditor;

    return () => {
      disposed = true;
      vditorReadyRef.current = false;
      try { vditor.destroy(); } catch {}
      vditorRef.current = null;
    };
  }, []);

  const handleSave = useCallback(async () => {
    if (!filePath || !dirty) return;
    try {
      await api.writeWikiFile(filePath, contentRef.current);
      setDirty(false);
    } catch (e) {
      setError(`保存失败: ${e}`);
    }
  }, [filePath, dirty]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        handleSave();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleSave]);

  const fileName = filePath ? filePath.split("/").pop() ?? filePath : null;

  return (
    <div className={styles.root}>
      {error && <div className={styles.error}>{error}</div>}
      {loading && <div className={styles.loading}>加载中...</div>}
      <div
        ref={containerRef}
        className={styles.host}
      />
      <div className={styles.statusBar}>
        <span className={styles.filePath} title={filePath ?? undefined}>
          {fileName ?? "未选择文件"}
        </span>
        <span className={styles.statusRight}>
          {dirty && <span className={styles.dirtyDot}>● 已修改</span>}
          <button
            type="button"
            disabled={!dirty}
            className={styles.saveBtn}
            onClick={handleSave}
          >
            保存
          </button>
        </span>
      </div>
    </div>
  );
}
