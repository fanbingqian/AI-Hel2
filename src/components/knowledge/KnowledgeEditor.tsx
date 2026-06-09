import { useState, useEffect, useCallback, useRef } from "react";
import { DocTree } from "./DocTree";
import { VditorEditor } from "./VditorEditor";
import { CherryEditor } from "./CherryEditor";
import { FilePreview } from "./FilePreview";
import { EntityBrowser } from "./EntityBrowser";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { uploadWikiFiles, nexusExtractFromFile, nexusSummarizeDocument, nexusDescribeImages, nexusAutoClassify } from "../../services/api";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import styles from "./KnowledgeEditor.module.css";

interface Props {
  onCanvasOpen?: (path: string) => void;
}

type EditorTab = "docs" | "entities";

export default function KnowledgeEditor({ onCanvasOpen }: Props) {
  const [tab, setTab] = useState<EditorTab>("docs");
  const [openFilePath, setOpenFilePath] = useState<string | null>(null);
  const [openFileKind, setOpenFileKind] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [uploadMsg, setUploadMsg] = useState<string | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(220);
  const [treeRefreshKey, setTreeRefreshKey] = useState(0);

  const handleDocSaved = useCallback(() => {
    setTreeRefreshKey((k) => k + 1);
  }, []);
  const sidebarRef = useRef(sidebarWidth);
  sidebarRef.current = sidebarWidth;
  const fetchGraphData = useKnowledgeStore((s) => s.fetchGraphData);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // ── Resize handle ──
  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarRef.current;
    const onMove = (ev: MouseEvent) => {
      const w = Math.max(150, Math.min(500, startW + (ev.clientX - startX)));
      setSidebarWidth(w);
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, []);

  // ── Sync chat to wiki ──
  const [syncChatOpen, setSyncChatOpen] = useState(false);
  const [sessions, setSessions] = useState<Array<{ id: string; title: string; messageCount: number }>>([]);
  const [syncingId, setSyncingId] = useState<string | null>(null);

  const handleSyncChatOpen = useCallback(async () => {
    try {
      const list = await invoke<Array<{ id: string; title: string; message_count: number }>>("list_sessions");
      setSessions(list.map((s) => ({ id: s.id, title: s.title, messageCount: s.message_count })));
      setSyncChatOpen(true);
    } catch { /* ignore */ }
  }, []);

  const handleSyncChat = useCallback(async (sessionId: string) => {
    setSyncingId(sessionId);
    try {
      const detail = await invoke<{ messages: Array<{ role: string; content: string }>; title?: string }>("get_session", { sessionId });
      const msgs = detail?.messages || [];
      if (msgs.length === 0) { setSyncingId(null); return; }

      const chatText = msgs.map((m) => `**${m.role === "user" ? "用户" : "AI"}**: ${m.content}`).join("\n\n");
      const sessionTitle = detail.title || sessions.find((s) => s.id === sessionId)?.title || "对话记录";

      await invoke("save_chat_to_knowledge", { sessionTitle, messagesJson: chatText, namespace: "chat" });
      setSyncChatOpen(false);
    } catch (e) {
      console.error("Sync chat failed:", e);
    } finally {
      setSyncingId(null);
    }
  }, [sessions]);

  const handleFileOpen = (path: string, fileKind?: string) => {
    setOpenFilePath(path);
    setOpenFileKind(fileKind ?? null);
  };

  const handleSync = () => {
    fetchGraphData();
  };

  // Auto-refresh graph data when entities are extracted from wiki files
  useEffect(() => {
    const u1 = listen("knowledge:extraction-complete", () => {
      fetchGraphData();
    });
    const u2 = listen("knowledge:graph-updated", () => {
      fetchGraphData();
    });
    return () => {
      u1.then((fn) => fn());
      u2.then((fn) => fn());
    };
  }, [fetchGraphData]);

  // Drag-and-drop file upload via Tauri window events
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWindow().onDragDropEvent(async (event) => {
      if (event.payload.type === "over") {
        setDragOver(true);
      } else if (event.payload.type === "leave") {
        setDragOver(false);
      } else if (event.payload.type === "drop") {
        setDragOver(false);
        const paths = event.payload.paths;
        if (paths.length > 0) {
          await handleUpload(paths);
        }
      }
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  const handleUpload = useCallback(async (paths: string[]) => {
    setUploading(true);
    setUploadMsg(null);
    try {
      const results = await uploadWikiFiles(paths);

      // Classify uploaded files
      const mdFiles: string[] = [];
      const docFiles: string[] = [];
      const imageFiles: string[] = [];

      for (const relPath of results) {
        const ext = relPath.split(".").pop()?.toLowerCase() || "";
        if (ext === "md") {
          mdFiles.push(relPath);
        } else if (["pdf", "docx", "pptx", "xlsx"].includes(ext)) {
          docFiles.push(relPath);
        } else if (["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp"].includes(ext)) {
          imageFiles.push(relPath);
        }
      }

      // Process markdown files: extract entities
      for (const relPath of mdFiles) {
        try { await nexusExtractFromFile(relPath); } catch { /* non-fatal */ }
      }

      // Process documents: auto-classify → summarize → save .md
      const newMdPaths: string[] = [];
      const failedSummaries: string[] = [];
      const classified: string[] = [];

      for (const relPath of docFiles) {
        const fileName = relPath.split("/").pop() || relPath;
        try {
          // Step 1: Auto-classify and archive
          setUploadMsg(`正在分类 ${fileName}...`);
          const classification = await nexusAutoClassify(relPath);
          const classifiedPath = classification.file_path || relPath;

          if (!classification.skipped) {
            classified.push(`${fileName} → ${classification.folder || "根目录"}`);
          }

          // Step 2: Summarize the classified file
          setUploadMsg(`正在生成 ${fileName} 的摘要...`);
          const mdPath = await nexusSummarizeDocument(classifiedPath);
          newMdPaths.push(mdPath);
        } catch (e) {
          const errMsg = String(e);
          failedSummaries.push(`${fileName}: ${errMsg}`);
          console.error(`Processing failed for ${relPath}:`, e);
        }
      }

      // Process images: auto-classify → describe
      let imageDescriptionFailed: string | null = null;
      if (imageFiles.length === 1) {
        try {
          setUploadMsg("正在分类图片...");
          const classification = await nexusAutoClassify(imageFiles[0]);
          const imgPath = classification.file_path || imageFiles[0];
          if (!classification.skipped) {
            classified.push(`${imageFiles[0].split("/").pop()} → ${classification.folder || "根目录"}`);
          }

          setUploadMsg("正在分析图片...");
          const mdPath = await nexusDescribeImages([imgPath]);
          newMdPaths.push(mdPath);
        } catch (e) {
          imageDescriptionFailed = String(e);
          console.error(`Image description failed:`, e);
        }
      } else if (imageFiles.length > 1) {
        try {
          setUploadMsg(`正在分类 ${imageFiles.length} 张图片...`);
          const classifiedPaths: string[] = [];
          for (const imgPath of imageFiles) {
            try {
              const classification = await nexusAutoClassify(imgPath);
              classifiedPaths.push(classification.file_path || imgPath);
              if (!classification.skipped) {
                classified.push(`${imgPath.split("/").pop()} → ${classification.folder || "根目录"}`);
              }
            } catch {
              classifiedPaths.push(imgPath); // Use original path on failure
            }
          }

          setUploadMsg(`正在分析 ${imageFiles.length} 张图片...`);
          const firstStem = imageFiles[0].split("/").pop()?.replace(/\.[^.]+$/, "") || "图集";
          const mdPath = await nexusDescribeImages(classifiedPaths, firstStem);
          newMdPaths.push(mdPath);
        } catch (e) {
          imageDescriptionFailed = String(e);
          console.error(`Multi-image description failed:`, e);
        }
      }

      // Extract entities from newly generated .md files
      for (const mdPath of newMdPaths) {
        try { await nexusExtractFromFile(mdPath); } catch { /* non-fatal */ }
      }

      let msg = `已上传 ${results.length} 个文件`;
      if (classified.length > 0) {
        msg += `，归档 ${classified.length} 个`;
      }
      if (newMdPaths.length > 0) {
        msg += `，生成 ${newMdPaths.length} 份摘要`;
      }
      if (failedSummaries.length > 0) {
        msg += ` | 失败: ${failedSummaries[0]}`;
        if (failedSummaries.length > 1) msg += ` 等${failedSummaries.length}项`;
      }
      if (imageDescriptionFailed) {
        msg += ` | 图片分析失败`;
      }
      setUploadMsg(msg);
      fetchGraphData();
    } catch (e) {
      setUploadMsg(`上传失败: ${e}`);
    } finally {
      setUploading(false);
      setTimeout(() => setUploadMsg(null), 4000);
    }
  }, [fetchGraphData]);

  const handleFilePicker = useCallback(async () => {
    const selected = await open({
      multiple: true,
      filters: [{
        name: "文档和图片",
        extensions: ["md", "canvas", "json", "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "docx", "xlsx", "pptx", "pdf"],
      }],
    });
    if (selected) {
      const paths = Array.isArray(selected) ? selected : [selected];
      await handleUpload(paths);
    }
  }, [handleUpload]);

  const handleFolderPicker = useCallback(async () => {
    const selected = await open({
      directory: true,
      multiple: false,
    });
    if (selected) {
      // For folders, we use the upload_folder approach — but since our command
      // takes file paths, we let the user pick files individually for now.
      // The folder picker shows what directory the user wants to upload to.
      setUploadMsg("请拖拽文件到此区域或点击上传按钮选择文件");
      setTimeout(() => setUploadMsg(null), 3000);
    }
  }, []);

  return (
    <div className={styles.editor}>
      <div className={styles.topBar}>
        <div className={styles.tabs}>
          <button
            className={`${styles.tab} ${tab === "docs" ? styles.activeTab : ""}`}
            onClick={() => setTab("docs")}
          >
            文档列表
          </button>
          <button
            className={`${styles.tab} ${tab === "entities" ? styles.activeTab : ""}`}
            onClick={() => setTab("entities")}
          >
            实体浏览
          </button>
        </div>
        <div className={styles.topActions}>
          {tab === "docs" && (
            <>
              <button className={styles.uploadBtn} onClick={handleFilePicker} disabled={uploading}>
                {uploading ? "上传中..." : "上传文件"}
              </button>
            </>
          )}
          <button className={styles.uploadBtn} onClick={handleSyncChatOpen}>
            同步对话
          </button>
          <button className={styles.syncBtn} onClick={handleSync}>
            同步到球体
          </button>
        </div>
      </div>

      {uploadMsg && (
        <div className={`${styles.uploadToast} ${uploadMsg.includes("失败") ? styles.toastError : ""}`}>
          {uploadMsg}
        </div>
      )}

      <div className={styles.body}>
        {tab === "docs" ? (
          <div className={styles.docsLayout}>
            <div style={{ width: sidebarWidth, minWidth: sidebarWidth, flexShrink: 0, display: "flex", flexDirection: "column" }}>
              <DocTree
                onFileOpen={handleFileOpen}
                onCanvasOpen={onCanvasOpen}
                onUploadClick={handleFilePicker}
                onRename={(oldPath, newPath) => {
                  if (openFilePath === oldPath) {
                    setOpenFilePath(newPath);
                  }
                }}
                refreshKey={treeRefreshKey}
              />
            </div>
            <div className={styles.resizeHandle} onMouseDown={handleResizeStart} />
            {openFilePath ? (
              openFilePath.endsWith(".md") || openFileKind === "md" ? (
                <CherryEditor filePath={openFilePath} onFileOpen={handleFileOpen} onSaved={handleDocSaved} />
              ) : (
                <FilePreview
                  filePath={openFilePath}
                  fileKind={openFileKind}
                  fileName={openFilePath.split("/").pop() || openFilePath}
                />
              )
            ) : (
              <div className={styles.emptyState}>选择左侧文件开始编辑</div>
            )}
          </div>
        ) : (
          <EntityBrowser />
        )}
      </div>

      {/* Drag-and-drop overlay */}
      {dragOver && (
        <div className={styles.dropOverlay}>
          <div className={styles.dropBox}>
            <span className={styles.dropIcon}>+</span>
            <span className={styles.dropText}>释放文件以上传</span>
            <span className={styles.dropSub}>支持 Markdown、图片、Canvas、Office 文档</span>
          </div>
        </div>
      )}

      {/* Sync chat modal */}
      {syncChatOpen && (
        <div className={styles.modalOverlay} onClick={() => setSyncChatOpen(false)}>
          <div className={styles.modal} onClick={(e) => e.stopPropagation()}>
            <h3 className={styles.modalTitle}>同步对话到知识库</h3>
            {sessions.length === 0 ? (
              <p className={styles.modalEmpty}>暂无对话记录</p>
            ) : (
              <ul className={styles.sessionList}>
                {sessions.map((s) => (
                  <li key={s.id} className={styles.sessionItem}>
                    <span className={styles.sessionTitle}>{s.title || "未命名对话"}</span>
                    <span className={styles.sessionCount}>{s.messageCount} 条消息</span>
                    <button
                      className={styles.sessionSyncBtn}
                      disabled={syncingId === s.id}
                      onClick={() => handleSyncChat(s.id)}
                    >
                      {syncingId === s.id ? "同步中..." : "同步"}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <button className={styles.modalClose} onClick={() => setSyncChatOpen(false)}>关闭</button>
          </div>
        </div>
      )}
    </div>
  );
}
