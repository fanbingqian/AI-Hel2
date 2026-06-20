import { useCallback, useEffect, useState, useRef } from "react";
import { readWikiFileBase64 } from "../../services/wiki";
import { showInFolder } from "../../services/api";
import { PdfViewer } from "./PdfViewer";
import styles from "./FilePreview.module.css";

interface Props {
  filePath: string;
}

function computeFileKind(fileName: string): string {
  const ext = fileName.split(".").pop()?.toLowerCase() || "";
  return /^(png|jpg|jpeg|gif|svg|webp|bmp|ico|tiff)$/.test(ext) ? "image" :
    ext === "pdf" ? "pdf" :
    /^(docx|xlsx|pptx)$/.test(ext) ? "convertible" : "static";
}

export function FilePreview({ filePath }: Props) {
  const fileName = filePath.split("/").pop() || filePath;
  const fileKind = computeFileKind(fileName);
  const [base64, setBase64] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rotation, setRotation] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const dragging = useRef(false);
  const lastPos = useRef({ x: 0, y: 0 });
  const blobUrlRef = useRef<string | null>(null);

  const resetView = useCallback(() => { setRotation(0); setZoom(1); setPan({ x: 0, y: 0 }); }, []);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY < 0 ? 0.15 : -0.15;
    setZoom(z => Math.min(10, Math.max(0.1, z + delta)));
  }, []);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if (zoom <= 1) return;
    dragging.current = true;
    lastPos.current = { x: e.clientX, y: e.clientY };
    e.currentTarget.setAttribute("style", "cursor:grabbing");
  }, [zoom]);

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (!dragging.current) return;
    const dx = e.clientX - lastPos.current.x;
    const dy = e.clientY - lastPos.current.y;
    lastPos.current = { x: e.clientX, y: e.clientY };
    setPan(p => ({ x: p.x + dx, y: p.y + dy }));
  }, []);

  const handleMouseUp = useCallback(() => {
    dragging.current = false;
  }, []);

  const handleDoubleClick = useCallback(() => {
    if (zoom > 1) { resetView(); } else { setZoom(2); setPan({ x: 0, y: 0 }); }
  }, [zoom, resetView]);

  useEffect(() => {
    return () => {
      // Clean up blob URL on unmount
      if (blobUrlRef.current) {
        URL.revokeObjectURL(blobUrlRef.current);
        blobUrlRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    setError(null);
    setBase64(null);
    if (blobUrlRef.current) {
      URL.revokeObjectURL(blobUrlRef.current);
      blobUrlRef.current = null;
    }

    if (fileKind === "image" || fileKind === "pdf") {
      readWikiFileBase64(filePath)
        .then((b64) => setBase64(b64))
        .catch((e) => setError(`加载失败: ${e}`));
    }
  }, [filePath, fileKind]);

  const ext = fileName.split(".").pop()?.toLowerCase() || "";
  const mimeMap: Record<string, string> = {
    pdf: "application/pdf",
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    gif: "image/gif",
    svg: "image/svg+xml",
    webp: "image/webp",
    bmp: "image/bmp",
    ico: "image/x-icon",
    tiff: "image/tiff",
  };
  const mime = mimeMap[ext] || "application/octet-stream";

  const kindLabels: Record<string, string> = {
    convertible: "Office 文档",
    canvas: "画板文件",
    pdf: "PDF 文档",
    image: "图片",
    static: "文件",
  };
  const kindLabel = kindLabels[fileKind || ""] || "文件";

  const iconMap: Record<string, string> = {
    convertible: "📑",
    canvas: "🎨",
    pdf: "📕",
    image: "🖼",
    static: "📄",
  };
  const icon = iconMap[fileKind || ""] || "📄";

  const handleOpenExternally = () => {
    // Use Tauri shell open to launch the file with system default app
    showInFolder(filePath).catch(() => {});
  };

  if (error) {
    return (
      <div className={styles.wrapper}>
        <div className={styles.fileCard}>
          <div className={styles.fileCardIcon}>{icon}</div>
          <div className={styles.fileCardName}>{fileName}</div>
          <div className={styles.fileCardError}>{error}</div>
          <button className={styles.fileCardBtn} type="button" onClick={handleOpenExternally}>
            在资源管理器中显示
          </button>
        </div>
      </div>
    );
  }

  if (fileKind === "pdf" && base64) {
    // Decode base64 to binary for PDF.js canvas rendering
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return (
      <div className={styles.wrapper}>
        <PdfViewer data={bytes} />
      </div>
    );
  }

  if (fileKind === "image" && base64) {
    return (
      <div className={styles.wrapper}>
        <div
          className={styles.imageContainer}
          onWheel={handleWheel}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
          onDoubleClick={handleDoubleClick}
          style={{ cursor: zoom > 1 ? "grab" : "default" }}
        >
          <img
            className={styles.image}
            src={`data:${mime};base64,${base64}`}
            alt={fileName}
            draggable={false}
            style={{ transform: `rotate(${rotation}deg) scale(${zoom}) translate(${pan.x}px,${pan.y}px)` }}
          />
        </div>
        <div className={styles.toolbar}>
          <button className={styles.toolBtn} onClick={() => setRotation(r => r - 90)} title="左转">↺</button>
          <button className={styles.toolBtn} onClick={resetView} title="重置">⟲</button>
          <button className={styles.toolBtn} onClick={() => setRotation(r => r + 90)} title="右转">↻</button>
        </div>
      </div>
    );
  }

  if (fileKind === "image" || fileKind === "pdf") {
    return (
      <div className={styles.wrapper}>
        <div className={styles.fileCard}>
          <div className={styles.fileCardIcon}>{icon}</div>
          <div className={styles.fileCardName}>{fileName}</div>
          <div className={styles.fileCardType}>加载中...</div>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.wrapper}>
      <div className={styles.fileCard}>
        <div className={styles.fileCardIcon}>{icon}</div>
        <div className={styles.fileCardName}>{fileName}</div>
        <div className={styles.fileCardType}>{kindLabel}</div>
        <button
          className={styles.fileCardBtn}
          type="button"
          onClick={handleOpenExternally}
        >
          在资源管理器中显示
        </button>
      </div>
    </div>
  );
}
