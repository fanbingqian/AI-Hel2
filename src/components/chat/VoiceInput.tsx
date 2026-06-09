import { useEffect, useRef, useState, useCallback } from "react";
import { useVoiceStore } from "../../stores/voiceStore";
import styles from "./VoiceInput.module.css";

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(Math.floor((seconds % 1) * 10))}`;
}

export function VoiceInput() {
  const status = useVoiceStore((s) => s.status);
  const transcribedText = useVoiceStore((s) => s.transcribedText);
  const startRecording = useVoiceStore((s) => s.startRecording);
  const stopRecording = useVoiceStore((s) => s.stopRecording);
  const cancelRecording = useVoiceStore((s) => s.cancelRecording);
  const confirmAndSend = useVoiceStore((s) => s.confirmAndSend);

  const [editedText, setEditedText] = useState("");
  const [duration, setDuration] = useState(0);
  const [waveHeights, setWaveHeights] = useState<number[]>(Array.from({ length: 16 }, () => 5));
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const waveRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pointerUsedRef = useRef(false);

  // Timer + waveform during listening
  useEffect(() => {
    if (status === "listening") {
      setDuration(0);
      timerRef.current = setInterval(() => {
        setDuration((d) => d + 1);
      }, 1000);
      waveRef.current = setInterval(() => {
        setWaveHeights(Array.from({ length: 16 }, () => 4 + Math.random() * 20));
      }, 100);
    } else {
      if (timerRef.current) { clearInterval(timerRef.current); timerRef.current = null; }
      if (waveRef.current) { clearInterval(waveRef.current); waveRef.current = null; }
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
      if (waveRef.current) clearInterval(waveRef.current);
    };
  }, [status]);

  // Load transcribed text into editor when preview starts
  useEffect(() => {
    if (status === "preview" && transcribedText) {
      setEditedText(transcribedText);
    }
  }, [status, transcribedText]);

  // Escape key to cancel
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") {
      if (status === "listening") cancelRecording();
      else if (status === "preview") {
        useVoiceStore.setState({ status: "idle", transcribedText: "", voiceText: "", isListening: false });
      }
    }
  }, [status, cancelRecording]);

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  // Hold-to-record (微信式): pointer down starts, pointer up stops.
  // Tap (quick press-and-release) also works via the same path:
  // start→record briefly→stop→transcribe.
  const handlePointerDown = (e: React.PointerEvent) => {
    e.preventDefault();
    pointerUsedRef.current = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    if (status === "idle") {
      startRecording("chat");
    }
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    e.preventDefault();
    if (status === "listening") {
      stopRecording();
    }
  };

  const handleSend = () => {
    const text = editedText.trim();
    if (text) {
      confirmAndSend(text);
      setEditedText("");
    }
  };

  const handleCancel = () => {
    useVoiceStore.setState({ status: "idle", transcribedText: "", voiceText: "", isListening: false });
    setEditedText("");
  };

  // ── Rendering by status ──

  if (status === "transcribing") {
    return (
      <div className={styles.wrap}>
        <div className={styles.transcribingHint}>
          <div className={styles.spinner} />
          识别中...
        </div>
      </div>
    );
  }

  if (status === "preview") {
    return (
      <div className={styles.wrap}>
        <div className={styles.previewWrap}>
          <textarea
            className={styles.previewInput}
            value={editedText}
            onChange={(e) => setEditedText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                handleSend();
              }
            }}
            rows={2}
            autoFocus
          />
          <div className={styles.previewActions}>
            <button className={`${styles.previewBtn} ${styles.cancelBtn}`} onClick={handleCancel}>✕</button>
            <button className={`${styles.previewBtn} ${styles.sendBtn}`} onClick={handleSend}>✓ 发送</button>
          </div>
        </div>
      </div>
    );
  }

  // idle or listening
  const isListening = status === "listening";

  return (
    <div className={styles.wrap}>
      <button
        className={`${styles.recordBtn} ${isListening ? styles.recordBtnActive : ""}`}
        onPointerDown={handlePointerDown}
        onPointerUp={handlePointerUp}
      >
        {isListening ? (
          <>
            <div className={styles.recordingIcon}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="#e74c3c">
                <rect x="9" y="2" width="6" height="12" rx="3" />
                <path d="M5 11a7 7 0 0 0 14 0" stroke="#e74c3c" strokeWidth="2" fill="none" />
              </svg>
            </div>
            <div className={styles.waveform}>
              {waveHeights.map((h, i) => (
                <div
                  key={i}
                  className={styles.waveBar}
                  style={{ height: h, opacity: 0.4 + (h / 24) * 0.6 }}
                />
              ))}
            </div>
            <div className={`${styles.timer} ${styles.timerActive}`}>
              {formatDuration(duration)}
            </div>
          </>
        ) : (
          <>
            <div className={styles.recordIcon}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <rect x="9" y="1" width="6" height="14" rx="3" />
                <path d="M5 10a7 7 0 0 0 14 0" />
              </svg>
            </div>
            点击或按住说话
          </>
        )}
      </button>
    </div>
  );
}
