import { useEffect, useRef, useState, useCallback, useId } from "react";
import Cherry from "cherry-markdown";
import "cherry-markdown/dist/cherry-markdown.css";
import { emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import * as api from "../../services/api";
import { readWikiFileBase64 } from "../../services/wiki";
import styles from "./CherryEditor.module.css";

type EditorMode = "editOnly" | "previewOnly" | "edit&preview";
type Theme = "light" | "dark";

interface Props {
  filePath: string | null;
  onFileOpen?: (path: string, fileKind?: string) => void;
  onSaved?: () => void;
}

function getSystemTheme(): Theme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

// ── Wikilink custom syntax hook ──
// Matches Obsidian-style [[target]] and [[target|alias]] wikilinks.
// Obsidian wikilink reference: https://help.obsidian.md/links
const WIKILINK_REGEX = /\[\[([^\]]+)\]\]/g;
const WikilinkHook = Cherry.createSyntaxHook("wikilink", "sentence", {
  test(str: string) {
    return WIKILINK_REGEX.test(str);
  },
  rule() {
    return {
      begin: "[[",
      content: "([^\\]]+)",
      end: "]]",
      reg: WIKILINK_REGEX,
    };
  },
  makeHtml(str: string) {
    return str.replace(WIKILINK_REGEX, (_match: string, inner: string) => {
      const pipeIdx = inner.lastIndexOf("|");
      const target = pipeIdx >= 0 ? inner.slice(0, pipeIdx) : inner;
      const alias = pipeIdx >= 0 ? inner.slice(pipeIdx + 1) : inner;
      const escapedTarget = target.replace(/"/g, "&quot;");
      return `<a class="wikilink" data-target="${escapedTarget}" href="javascript:void(0)">${alias}</a>`;
    });
  },
});

export function CherryEditor({ filePath, onFileOpen, onSaved }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cherryRef = useRef<Cherry | null>(null);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<EditorMode>("edit&preview");
  const [theme, setThemeState] = useState<Theme>(getSystemTheme);
  const contentRef = useRef("");
  const instanceId = useId().replace(/:/g, "");

  // Load file content when filePath changes
  useEffect(() => {
    if (!filePath) {
      contentRef.current = "";
      setError(null);
      setLoading(false);
      setDirty(false);
      cherryRef.current?.setMarkdown("");
      return;
    }

    setLoading(true);
    setError(null);
    api
      .readWikiFile(filePath)
      .then((data) => {
        contentRef.current = data;
        setLoading(false);
        setDirty(false);
        if (cherryRef.current) {
          cherryRef.current.setMarkdown(data);
        }
      })
      .catch((e) => {
        setError(`读取失败: ${e}`);
        contentRef.current = "";
        setLoading(false);
      });
  }, [filePath]);

  // Create Cherry instance once
  useEffect(() => {
    if (!containerRef.current || cherryRef.current) return;

    const cherry = new Cherry({
      id: containerRef.current.id,
      value: contentRef.current,
      engine: {
        customSyntax: {
          wikilink: {
            syntaxClass: WikilinkHook,
            force: false,
          },
        },
      },
      editor: {
        defaultModel: "edit&preview",
        codemirror: {
          autofocus: true,
        },
      },
      toolbars: {
        toolbar: [
          "bold", "italic", "strikethrough", "|",
          "color", "header", "|",
          "list", "quote", "code", "|",
          {
            insert: ["image", "link", "table", "hr", "br", "toc"],
          },
          "graph", "togglePreview",
        ],
        bubble: ["bold", "italic", "underline", "strikethrough", "code", "link"],
        float: ["h1", "h2", "h3", "|", "checklist", "quote", "table", "code"],
      },
      themeSettings: {
        mainTheme: theme,
        codeBlockTheme: theme,
        inlineCodeTheme: "red",
        themeList: [
          { className: "light", label: "Light" },
          { className: "dark", label: "Dark" },
        ],
      },
      callback: {
        afterChange: (text: string) => {
          contentRef.current = text;
          setDirty(true);
        },
        onClickPreview: (e: MouseEvent) => {
          const target = e.target as HTMLElement;
          const link = target.closest("a.wikilink") as HTMLAnchorElement | null;
          if (link && onFileOpen) {
            e.preventDefault();
            const linkTarget = link.dataset.target;
            if (linkTarget) {
              // Resolve wikilink relative to current file's directory
              let resolved: string;
              if (filePath) {
                const dir = filePath.replace(/[/][^/]+$/, "");
                resolved = `${dir}/${linkTarget}`;
              } else {
                resolved = linkTarget;
              }
              // Add .md extension if no extension present
              if (!/\.[a-zA-Z0-9]+$/.test(resolved)) {
                resolved = `${resolved}.md`;
              }
              onFileOpen(resolved, "md");
            }
          }
        },
      },
    });

    cherryRef.current = cherry;

    return () => {
      cherry.destroy();
      cherryRef.current = null;
    };
  }, []);

  // ── Resolve relative image paths to base64 for webview display ──
  const resolveWikiImage = useCallback(
    (img: HTMLImageElement) => {
      const src = img.getAttribute("src") || "";
      // Only process relative paths — skip absolute URLs and data URIs
      if (!src || src.startsWith("http://") || src.startsWith("https://") || src.startsWith("data:")) {
        return;
      }
      // Resolve relative to current file's directory
      let resolvedPath: string;
      if (filePath) {
        const dir = filePath.replace(/[/][^/]+$/, "");
        resolvedPath = src.startsWith("/") ? src.slice(1) : `${dir}/${src}`;
      } else {
        resolvedPath = src.startsWith("/") ? src.slice(1) : src;
      }
      // Deduplicate: only resolve each path once per session
      const key = `cherry-img-${resolvedPath}`;
      const cached = (img as any).__cherryImageKey;
      if (cached === key) return;
      (img as any).__cherryImageKey = key;

      readWikiFileBase64(resolvedPath)
        .then((b64) => {
          const ext = src.split(".").pop()?.toLowerCase() || "png";
          const mimeMap: Record<string, string> = {
            png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg",
            gif: "image/gif", svg: "image/svg+xml", webp: "image/webp",
            bmp: "image/bmp", ico: "image/x-icon",
          };
          const mime = mimeMap[ext] || "image/png";
          img.src = `data:${mime};base64,${b64}`;
        })
        .catch(() => {
          // Leave src as-is if the image can't be loaded
        });
    },
    [filePath],
  );

  // Watch for new <img> elements in Cherry's preview DOM
  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;

    const scanImages = (root: HTMLElement) => {
      root.querySelectorAll("img").forEach((img) => resolveWikiImage(img as HTMLImageElement));
    };

    // Initial scan (Cherry may render after this effect runs)
    const initialTimer = setTimeout(() => scanImages(container), 200);

    const observer = new MutationObserver((mutations) => {
      for (const m of mutations) {
        for (const node of m.addedNodes) {
          if (node instanceof HTMLImageElement) {
            resolveWikiImage(node);
          } else if (node instanceof HTMLElement) {
            scanImages(node);
          }
        }
      }
    });

    observer.observe(container, { childList: true, subtree: true });

    return () => {
      observer.disconnect();
      clearTimeout(initialTimer);
    };
  }, [resolveWikiImage]);

  // Listen for system theme changes
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: MediaQueryListEvent) => {
      const newTheme: Theme = e.matches ? "dark" : "light";
      setThemeState(newTheme);
      cherryRef.current?.setTheme(newTheme);
    };
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  const handleSave = useCallback(async () => {
    if (!filePath || !dirty) return;
    try {
      // Prefer contentRef (raw text from afterChange, includes YAML frontmatter).
      // Cherry's getMarkdown() may strip frontmatter block.
      const currentContent = contentRef.current || cherryRef.current?.getMarkdown() || "";
      await api.writeWikiFile(filePath, currentContent);
      setDirty(false);
      onSaved?.();
      // Fire tree refresh immediately (FileWatcher also fires after debounce)
      emit("wiki:files-changed", { path: filePath, change_type: "Modified" });
    } catch (e) {
      setError(`保存失败: ${e}`);
    }
  }, [filePath, dirty, onSaved]);

  const handleModeSwitch = useCallback(
    (newMode: EditorMode) => {
      setMode(newMode);
      cherryRef.current?.switchModel(newMode);
    },
    []
  );

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

  // Paste/drop image insertion
  const insertImage = useCallback(async (blob: Blob, name: string) => {
    if (!filePath) return;
    const dir = filePath.replace(/\/[^/]+$/, "");
    const ext = name.split(".").pop() || "png";
    const ts = Date.now();
    const imgName = `${dir}/img_${ts}.${ext}`;
    try {
      const buf = await blob.arrayBuffer();
      const bytes = new Uint8Array(buf);
      let b64 = "";
      for (let i = 0; i < bytes.length; i++) b64 += String.fromCharCode(bytes[i]);
      b64 = btoa(b64);
      await invoke("write_wiki_file_base64", { path: imgName, data: b64 });
      const md = cherryRef.current?.getMarkdown() || "";
      cherryRef.current?.setMarkdown(md + `\n![${name}](${imgName})\n`);
      setDirty(true);
    } catch (e) {
      console.error("Insert image failed:", e);
    }
  }, [filePath]);

  useEffect(() => {
    if (!containerRef.current || !filePath) return;
    const el = containerRef.current;
    const onPaste = (e: ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      for (let i = 0; i < items.length; i++) {
        if (items[i].type.startsWith("image/")) {
          e.preventDefault();
          const blob = items[i].getAsFile();
          if (blob) insertImage(blob, `paste_${Date.now()}.png`);
          break;
        }
      }
    };
    const onDrop = (e: DragEvent) => {
      const files = e.dataTransfer?.files;
      if (!files) return;
      for (let i = 0; i < files.length; i++) {
        if (files[i].type.startsWith("image/")) {
          e.preventDefault();
          insertImage(files[i], files[i].name);
        }
      }
    };
    el.addEventListener("paste", onPaste);
    el.addEventListener("drop", onDrop);
    return () => { el.removeEventListener("paste", onPaste); el.removeEventListener("drop", onDrop); };
  }, [filePath, insertImage]);

  const fileName = filePath ? filePath.split("/").pop() ?? filePath : null;

  const modeLabel: Record<EditorMode, string> = {
    editOnly: "编辑",
    previewOnly: "阅读",
    "edit&preview": "分屏",
  };

  const nextMode: Record<EditorMode, EditorMode> = {
    "edit&preview": "editOnly",
    editOnly: "previewOnly",
    previewOnly: "edit&preview",
  };

  return (
    <div className={styles.root}>
      {error && <div className={styles.error}>{error}</div>}
      {loading && <div className={styles.loading}>加载中...</div>}
      <div
        ref={containerRef}
        id={`cherry-editor-${instanceId}`}
        className={styles.host}
      />
      <div className={styles.statusBar}>
        <div className={styles.statusRow}>
          <span className={styles.filePath} title={filePath ?? undefined}>
            {fileName ?? "未选择文件"}
          </span>
          <span className={styles.statusRight}>
            <button
              type="button"
              className={styles.modeBtn}
              title={`切换到${modeLabel[nextMode[mode]]}模式`}
              onClick={() => handleModeSwitch(nextMode[mode])}
            >
              {modeLabel[mode]}
            </button>
            <button
              type="button"
              className={styles.themeBtn}
              title={`切换到${theme === "dark" ? "亮色" : "暗色"}主题`}
              onClick={() => {
                const t = theme === "dark" ? "light" : "dark";
                setThemeState(t);
                cherryRef.current?.setTheme(t);
              }}
            >
              {theme === "dark" ? "☀" : "☾"}
            </button>
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
        {dirty && (
          <div className={`${styles.statusRow} ${styles.dirtyRow}`}>
            <span className={styles.dirtyDot}>● 已修改</span>
          </div>
        )}
      </div>
    </div>
  );
}
