import React from "react";
import { listen } from "@tauri-apps/api/event";

interface OverlayState {
  recording: boolean;
  duration: number;
}

export function VoiceOverlay() {
  const [state, setState] = React.useState<OverlayState>({
    recording: false,
    duration: 0,
  });
  const [waveHeights, setWaveHeights] = React.useState<number[]>(
    Array.from({ length: 12 }, () => 4)
  );
  const waveRef = React.useRef<ReturnType<typeof setInterval> | null>(null);

  // Transparent window background
  React.useEffect(() => {
    document.documentElement.style.cssText =
      "background:transparent!important;margin:0;padding:0;overflow:hidden;";
    document.body.style.cssText =
      "background:transparent!important;margin:0;padding:0;overflow:hidden;width:100%;height:100%;";
    const root = document.getElementById("root");
    if (root) {
      root.style.cssText =
        "background:transparent!important;width:100%;height:100%;display:flex;align-items:center;justify-content:center;";
    }
  }, []);

  // Listen for overlay state updates from Rust
  React.useEffect(() => {
    const unlisten = listen<OverlayState>("voice-overlay:state", (e) => {
      setState(e.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Animated waveform
  React.useEffect(() => {
    if (state.recording) {
      waveRef.current = setInterval(() => {
        const baseHeight = 4 + Math.random() * 20;
        setWaveHeights(
          Array.from({ length: 12 }, () => baseHeight * (0.3 + Math.random() * 0.7))
        );
      }, 80);
    } else {
      if (waveRef.current) {
        clearInterval(waveRef.current);
        waveRef.current = null;
      }
      setWaveHeights(Array.from({ length: 12 }, () => 4));
    }
    return () => {
      if (waveRef.current) clearInterval(waveRef.current);
    };
  }, [state.recording]);

  return (
    <div
      style={{
        width: 220,
        height: 220,
        borderRadius: "50%",
        background: state.recording
          ? "radial-gradient(circle, rgba(231, 76, 60, 0.88), rgba(231, 76, 60, 0.25))"
          : "radial-gradient(circle, rgba(7, 193, 96, 0.85), rgba(7, 193, 96, 0.2))",
        backdropFilter: "blur(20px)",
        WebkitBackdropFilter: "blur(20px)",
        border: "2px solid rgba(255, 255, 255, 0.25)",
        boxShadow: state.recording
          ? "0 0 50px rgba(231, 76, 60, 0.5), 0 0 100px rgba(231, 76, 60, 0.15)"
          : "0 0 50px rgba(7, 193, 96, 0.4), 0 0 80px rgba(7, 193, 96, 0.1)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 10,
        userSelect: "none" as const,
        cursor: "default",
        transition: "background 0.2s, box-shadow 0.2s",
      }}
    >
      {/* Mic icon */}
      <svg
        width="28"
        height="28"
        viewBox="0 0 24 24"
        fill={state.recording ? "#fff" : "rgba(255,255,255,0.85)"}
      >
        <rect x="9" y="1" width="6" height="13" rx="3" />
        <path
          d="M5 10a7 7 0 0 0 14 0"
          stroke={state.recording ? "#fff" : "rgba(255,255,255,0.85)"}
          strokeWidth="2"
          fill="none"
        />
      </svg>

      {/* Waveform */}
      <div style={{ display: "flex", alignItems: "center", gap: 3, height: 26 }}>
        {waveHeights.map((h, i) => (
          <div
            key={i}
            style={{
              width: 4,
              height: h,
              borderRadius: 2,
              background: "rgba(255,255,255,0.85)",
              transition: "height 0.08s ease",
            }}
          />
        ))}
      </div>

      {/* Status */}
      <span
        style={{
          color: "#fff",
          fontSize: 13,
          fontWeight: 600,
          textShadow: "0 1px 3px rgba(0,0,0,0.5)",
        }}
      >
        {state.recording ? "正在录音..." : "就绪"}
      </span>

      {state.recording && state.duration > 0 && (
        <span style={{ color: "rgba(255,255,255,0.6)", fontSize: 11 }}>
          {state.duration.toFixed(1)}s
        </span>
      )}
    </div>
  );
}
