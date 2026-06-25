import { useCallback, useState, useEffect, useRef } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { uploadWikiFiles } from "../../services/api";
import { useUIStore } from "../../stores/uiStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { DocTree } from "../knowledge/DocTree";
import { EntityBrowser } from "../knowledge/EntityBrowser";
import { ChatPanel } from "../chat/ChatPanel";
import { SessionList } from "../chat/SessionList";
import { PanelResizer } from "../layout/PanelResizer";
import { MainContent } from "./MainContent";
import styles from "./AiHelPage.module.css";

export default function AiHelPage() {
  const chatPanelWidth = useUIStore((s) => s.chatPanelWidth);
  const setChatPanelWidth = useUIStore((s) => s.setChatPanelWidth);
  const docTreeWidth = useUIStore((s) => s.docTreeWidth);
  const setDocTreeWidth = useUIStore((s) => s.setDocTreeWidth);
  const panelCollapsed = useUIStore((s) => s.panelCollapsed);
  const setPanelCollapsed = useUIStore((s) => s.setPanelCollapsed);
  const mainContentMode = useUIStore((s) => s.mainContentMode);
  const setMainContentMode = useUIStore((s) => s.setMainContentMode);
  const openFilePath = useUIStore((s) => s.openFilePath);
  const setOpenFilePath = useUIStore((s) => s.setOpenFilePath);
  const sessionListOpen = useUIStore((s) => s.sessionListExpanded);
  const [docTab, setDocTab] = useState<"docs" | "entities">("docs");

  // Initialize data loading (was in old SphereChatView)
  const fetchGraphData = useKnowledgeStore((s) => s.fetchGraphData);
  const loadInferences = useKnowledgeStore((s) => s.loadInferences);
  const loadGraphSettings = useKnowledgeStore((s) => s.loadGraphSettings);
  useEffect(() => { fetchGraphData(); loadInferences(); }, [fetchGraphData, loadInferences]);
  useEffect(() => { loadGraphSettings(); }, [loadGraphSettings]);

  // ── Dynamic window sizing on collapse/expand ──
  // Hidden: 9:16 (390×693), Expanded: 16:9 (1232×693), same short side
  // Both sizes are fixed — no tracking, no drift
  const CHAT_MIN_W = 340;
  const CHAT_MAX_W = 500;
  const EXPANDED_MIN_W = 900;
  const MIN_H = 500;

  const prevCollapsedRef = useRef(panelCollapsed);

  useEffect(() => {
    const win = getCurrentWindow();
    let cancelled = false;
    const changed = prevCollapsedRef.current !== panelCollapsed;
    prevCollapsedRef.current = panelCollapsed;

    (async () => {
      try {
        if (!changed) {
          // Mount: set constraints only, Tauri already created correct-size window
          if (panelCollapsed) {
            await win.setMinSize(new LogicalSize(CHAT_MIN_W, MIN_H));
            await win.setMaxSize(new LogicalSize(CHAT_MAX_W, 4000));
          } else {
            await win.setMinSize(new LogicalSize(EXPANDED_MIN_W, MIN_H));
          }
          return;
        }

        if (panelCollapsed) {
          // ── COLLAPSE → 390×693 fixed ──
          if (cancelled) return;
          await win.setMinSize(new LogicalSize(CHAT_MIN_W, MIN_H));
          await win.setMaxSize(new LogicalSize(CHAT_MAX_W, 4000));
          await win.setSize(new LogicalSize(390, 693));
          await win.setFocus();
        } else {
          // ── EXPAND → 1232×693 fixed ──
          if (cancelled) return;
          await win.setMaxSize(new LogicalSize(4000, 4000));
          await win.setMinSize(new LogicalSize(EXPANDED_MIN_W, MIN_H));
          await win.setSize(new LogicalSize(1232, 693));
          await win.setFocus();
        }
      } catch (err) {
        console.error("[AiHelPage window resize error]", err);
      }
    })();

    return () => { cancelled = true; };
  }, [panelCollapsed]);

  // Double-click entity → navigate to detail page
  const navEntityId = useKnowledgeStore((s) => s.navEntityId);
  useEffect(() => {
    if (navEntityId) {
      setMainContentMode("entity");
      useKnowledgeStore.getState().setNavEntityId(null);
    }
  }, [navEntityId, setMainContentMode]);

  const handleUploadClick = useCallback(async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: "文档和图片", extensions: ["md", "canvas", "json", "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "docx", "xlsx", "pptx", "pdf"] }],
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        await uploadWikiFiles(paths);
      }
    } catch { /* user cancelled */ }
  }, []);

  const handleFileOpen = useCallback(
    (path: string, fileKind?: string) => {
      setOpenFilePath(path);
      if (path.endsWith(".canvas")) {
        setMainContentMode("canvas");
      } else if (path.endsWith(".md") || fileKind === "md") {
        setMainContentMode("editor");
      } else {
        setMainContentMode("preview");
      }
    },
    [setOpenFilePath, setMainContentMode],
  );

  const handleCanvasOpen = useCallback(
    (path: string) => {
      setOpenFilePath(path);
      setMainContentMode("canvas");
    },
    [setOpenFilePath, setMainContentMode],
  );

  const handleDocTreeResize = useCallback(
    (delta: number) => setDocTreeWidth(docTreeWidth + delta),
    [docTreeWidth, setDocTreeWidth],
  );

  const handleChatResize = useCallback(
    (delta: number) => setChatPanelWidth(chatPanelWidth - delta),
    [chatPanelWidth, setChatPanelWidth],
  );

  return (
    <div className={styles.container}>
      {/* Left panel – always mounted, hidden via CSS when collapsed */}
      <div className={styles.docTreeCol} style={{ width: panelCollapsed ? 0 : docTreeWidth, minWidth: 0, overflow: "hidden", borderRight: panelCollapsed ? "none" : undefined }}>
        <div className={styles.docTabs}>
          <button type="button" className={`${styles.docTab} ${docTab === "docs" ? styles.docTabActive : ""}`} onClick={() => setDocTab("docs")}>文档列表</button>
          <button type="button" className={`${styles.docTab} ${docTab === "entities" ? styles.docTabActive : ""}`} onClick={() => setDocTab("entities")}>实体列表</button>
        </div>
        <div style={{ display: docTab === "docs" ? "flex" : "none", flexDirection: "column", flex: 1, minHeight: 0 }}>
          <DocTree onFileOpen={handleFileOpen} onCanvasOpen={handleCanvasOpen} onUploadClick={handleUploadClick} />
        </div>
        <div style={{ display: docTab === "entities" ? "flex" : "none", flexDirection: "column", flex: 1, minHeight: 0 }}>
          <EntityBrowser compact />
        </div>
      </div>
      {!panelCollapsed && <PanelResizer onResize={handleDocTreeResize} />}

      {/* Main panel – always mounted, hidden via CSS when collapsed */}
      <div className={styles.mainCol} style={{ flex: panelCollapsed ? "0 0 0" : 1, minWidth: 0, overflow: "hidden" }}>
        <MainContent
          mode={mainContentMode}
          filePath={openFilePath}
          onFileOpen={handleFileOpen}
          onCloseFile={() => { setOpenFilePath(null); setMainContentMode("graph2d"); }}
        />
      </div>
      {!panelCollapsed && <PanelResizer onResize={handleChatResize} />}

      {/* Chat – fills window when collapsed, fixed width when expanded */}
      <div className={styles.chatCol} style={panelCollapsed ? { flex: 1 } : undefined}>
        <ChatPanel />
        {sessionListOpen && <SessionList />}
      </div>
    </div>
  );
}
