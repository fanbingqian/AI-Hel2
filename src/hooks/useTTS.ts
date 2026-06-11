import { useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const MAX_SEGMENT_CHARS = 200;

function splitIntoSegments(text: string): string[] {
  const segments: string[] = [];
  let remaining = text.trim();
  while (remaining.length > 0) {
    if (remaining.length <= MAX_SEGMENT_CHARS) {
      segments.push(remaining);
      break;
    }
    const chunk = remaining.slice(0, MAX_SEGMENT_CHARS);
    const boundary = Math.max(
      chunk.lastIndexOf("。"),
      chunk.lastIndexOf("！"),
      chunk.lastIndexOf("？"),
      chunk.lastIndexOf("."),
      chunk.lastIndexOf("!"),
      chunk.lastIndexOf("?"),
    );
    if (boundary > MAX_SEGMENT_CHARS / 3) {
      segments.push(remaining.slice(0, boundary + 1));
      remaining = remaining.slice(boundary + 1).trim();
    } else {
      segments.push(chunk);
      remaining = remaining.slice(MAX_SEGMENT_CHARS).trim();
    }
  }
  return segments.filter((s) => s.length > 0);
}

export function useTTS() {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const playingRef = useRef(false);
  const cancelledRef = useRef(false);

  const stop = useCallback(() => {
    cancelledRef.current = true;
    if (audioRef.current) {
      audioRef.current.pause();
      URL.revokeObjectURL(audioRef.current.src);
      audioRef.current = null;
    }
    playingRef.current = false;
  }, []);

  const playBase64 = useCallback(async (base64: string): Promise<void> => {
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const blob = new Blob([bytes], { type: "audio/wav" });
    const url = URL.createObjectURL(blob);
    const audio = new Audio(url);
    audioRef.current = audio;
    playingRef.current = true;

    return new Promise<void>((resolve) => {
      audio.onended = () => {
        playingRef.current = false;
        URL.revokeObjectURL(url);
        resolve();
      };
      audio.onerror = () => {
        playingRef.current = false;
        URL.revokeObjectURL(url);
        resolve();
      };
      audio.play().catch(() => resolve());
    });
  }, []);

  const speak = useCallback(async (text: string, speakerId?: number) => {
    stop();
    cancelledRef.current = false;
    try {
      const voice = speakerId !== undefined ? String(speakerId) : undefined;
      const base64: string = await invoke("tts_speak", { text, voice });
      if (!cancelledRef.current) {
        await playBase64(base64);
      }
    } catch (e) {
      console.error("TTS speak failed:", e);
      playingRef.current = false;
    }
  }, [stop, playBase64]);

  const speakSegments = useCallback(async (text: string, speakerId?: number) => {
    stop();
    cancelledRef.current = false;

    const segments = splitIntoSegments(text);
    const voice = speakerId !== undefined ? String(speakerId) : undefined;

    for (let i = 0; i < segments.length; i++) {
      if (cancelledRef.current) break;
      const segment = segments[i];
      try {
        const base64: string = await invoke("tts_speak", { text: segment, voice });
        if (cancelledRef.current) break;
        await playBase64(base64);
      } catch (e) {
        console.error("TTS segment failed:", e);
        break;
      }
    }

    playingRef.current = false;
  }, [stop, playBase64]);

  const isSpeaking = () => playingRef.current;

  return { speak, speakSegments, stop, isSpeaking };
}
