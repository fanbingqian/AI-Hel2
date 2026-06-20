import { useEffect, useRef, useState } from "react";
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
  const [scale, setScale] = useState(1.0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totalPages, setTotalPages] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setPages([]);
    setTotalPages(0);

    (async () => {
      try {
        const pdf = await pdfjsLib.getDocument({ data }).promise;
        if (cancelled) return;
        setTotalPages(pdf.numPages);
        // Determine fit-to-width scale based on container width
        const containerWidth = containerRef.current?.clientWidth || 800;
        const firstPage = await pdf.getPage(1);
        const baseViewport = firstPage.getViewport({ scale: 1 });
        const fitScale = (containerWidth - 32) / baseViewport.width; // 32px padding
        const effectiveScale = scale > 0.5 ? scale : Math.min(fitScale, 1.5);
        setScale(effectiveScale);

        const pageData: Uint8Array[] = [];
        for (let i = 1; i <= pdf.numPages; i++) {
          const page = await pdf.getPage(i);
          const viewport = page.getViewport({ scale: effectiveScale });
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

  // Ctrl+wheel = zoom; normal wheel = scroll (browser default)
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        setScale(s => Math.min(5, Math.max(0.25, s + (e.deltaY < 0 ? 0.2 : -0.2))));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
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
    <div style={{ flex: 1, position: "relative", overflow: "hidden", display: "flex", flexDirection: "column" }}>
      <div
        ref={containerRef}
        style={{
          flex: 1,
          overflow: "auto",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 8,
          padding: "12px 0",
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
              style={{ maxWidth: `${scale * 100}%`, height: "auto", display: "block", boxShadow: "0 1px 3px rgba(0,0,0,0.3)" }}
              onLoad={() => URL.revokeObjectURL(url)}
            />
          );
        })}
      </div>
      {/* Floating toolbar */}
      <div style={{
        position: "absolute", bottom: 12, left: "50%", transform: "translateX(-50%)",
        display: "flex", gap: 6, zIndex: 10, opacity: 0, transition: "opacity 0.2s",
        background: "rgba(0,0,0,0.65)", padding: "6px 10px", borderRadius: 8,
      }} className="pdf-toolbar"
        onMouseEnter={e => (e.currentTarget.style.opacity = "1")}
        onMouseLeave={e => (e.currentTarget.style.opacity = "0")}>
        <button onClick={() => setScale(s => Math.max(0.5, s - 0.25))}
          style={btnStyle}>−</button>
        <span style={{ color: "#e0e0e0", fontSize: 12, minWidth: 48, textAlign: "center" }}>
          {Math.round(scale * 100)}%
        </span>
        <button onClick={() => setScale(s => Math.min(5, s + 0.25))}
          style={btnStyle}>+</button>
        {totalPages > 1 && (
          <span style={{ color: "#808080", fontSize: 11, marginLeft: 8 }}>{totalPages} 页</span>
        )}
      </div>
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  padding: "2px 8px", background: "rgba(255,255,255,0.1)", color: "#e0e0e0",
  border: "1px solid rgba(255,255,255,0.15)", borderRadius: 4,
  cursor: "pointer", fontSize: 14, fontFamily: "inherit",
};
