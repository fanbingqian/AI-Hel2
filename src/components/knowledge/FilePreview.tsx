import { useEffect, useState, useRef } from "react";
import { readWikiFileBase64 } from "../../services/wiki";
import { showInFolder } from "../../services/api";
import styles from "./FilePreview.module.css";

interface Props {
  filePath: string;
  fileKind: string | null;
  fileName: string;
}

function base64ToBlobUrl(base64: string, mime: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  const blob = new Blob([bytes], { type: mime });
  return URL.createObjectURL(blob);
}

export function FilePreview({ filePath, fileKind, fileName }: Props) {
  const [base64, setBase64] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const blobUrlRef = useRef<string | null>(null);

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
    // Use Blob URL instead of data: URL — Chromium blocks data: URLs for PDF in iframes
    const blobUrl = base64ToBlobUrl(base64, mime);
    return (
      <div className={styles.wrapper}>
        <iframe
          className={styles.pdfFrame}
          src={blobUrl}
          title={fileName}
        />
      </div>
    );
  }

  if (fileKind === "image" && base64) {
    return (
      <div className={styles.wrapper}>
        <div className={styles.imageContainer}>
          <img
            className={styles.image}
            src={`data:${mime};base64,${base64}`}
            alt={fileName}
          />
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
