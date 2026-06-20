import { useCallback, useEffect, useRef, useState } from "react";
import * as pdfjsLib from "pdfjs-dist";

// Set worker path to the bundled worker
pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url
).toString();

interface Props {
  data: Uint8Array;
}

export function PdfViewer({ data }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [pages, setPages] = useState<Uint8Array[]>([]);
  const [scale, setScale] = useState(1.5);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    (async () => {
      try {
        const pdf = await pdfjsLib.getDocument({ data }).promise;
        if (cancelled) return;
        const pageData: Uint8Array[] = [];
        for (let i = 1; i <= pdf.numPages; i++) {
          const page = await pdf.getPage(i);
          const viewport = page.getViewport({ scale: 1.5 });
          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          const ctx = canvas.getContext("2d")!;
          await page.render({ canvas, canvasContext: ctx, viewport }).promise;
          canvas.toBlob((blob) => {
            if (blob && !cancelled) {
              blob.arrayBuffer().then((buf) => {
                pageData[i - 1] = new Uint8Array(buf);
                if (pageData.filter(Boolean).length === pdf.numPages && !cancelled) {
                  setPages([...pageData]);
                  setLoading(false);
                }
              });
            }
          }, "image/png");
        }
      } catch (e: any) {
        if (!cancelled) {
          setError(e?.message || "PDF 加载失败");
          setLoading(false);
        }
      }
    })();

    return () => { cancelled = true; };
  }, [data]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    setScale(s => Math.min(5, Math.max(0.5, s + (e.deltaY < 0 ? 0.25 : -0.25))));
  }, []);

  if (loading) {
    return (
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: "#808080", fontSize: 14 }}>
        PDF 加载中...
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: "#ef4444", fontSize: 14 }}>
        {error}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      onWheel={handleWheel}
      style={{
        flex: 1,
        overflow: "auto",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 4,
        padding: "8px 0",
      }}
    >
      {pages.map((pageData, i) => {
        const blob = new Blob([pageData.buffer as ArrayBuffer], { type: "image/png" });
        const url = URL.createObjectURL(blob);
        return (
          <img
            key={i}
            src={url}
            alt={`第 ${i + 1} 页`}
            style={{ maxWidth: "100%", height: "auto", transform: `scale(${scale})`, transformOrigin: "top center" }}
            onLoad={() => URL.revokeObjectURL(url)}
          />
        );
      })}
      {pages.length > 1 && (
        <div style={{ padding: "8px", color: "#808080", fontSize: 11 }}>共 {pages.length} 页 · 滚轮缩放</div>
      )}
    </div>
  );
}
