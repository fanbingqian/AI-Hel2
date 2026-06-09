import { useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

const MAX_SEGMENT_CHARS = 200;

function splitIntoSegments(text: string): string[] {
  const segments: string[] = [];
  let remaining = text.trim();
  while (remaining.length > 0) {
    if (remaining.length <= MAX_SEGMENT_CHARS) {
      segments.push(remaining);
      break;
    }
    // Find the last sentence boundary within the limit
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
      // No good boundary — break at MAX_SEGMENT_CHARS
      segments.push(chunk);
      remaining = remaining.slice(MAX_SEGMENT_CHARS).trim();
    }
  }
  return segments.filter((s) => s.length > 0);
}

const BARGE_IN_GRACE_MS = 400;

export function useTTS() {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const playingRef = useRef(false);
  const cancelledRef = useRef(false);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const playStartTimeRef = useRef(0);

  const stop = useCallback(() => {
    cancelledRef.current = true;
    if (audioRef.current) {
      audioRef.current.pause();
      URL.revokeObjectURL(audioRef.current.src);
      audioRef.current = null;
    }
    playingRef.current = false;
    invoke("voice_stop_barge_in_monitor").catch(() => {});
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
  }, []);

  const playBase64 = useCallback(async (base64: string): Promise<void> => {
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const blob = new Blob([bytes], { type: "audio/wav" });
    const url = URL.createObjectURL(blob);
    const audio = new Audio(url);
    audioRef.current = audio;
    playingRef.current = true;
    playStartTimeRef.current = Date.now();

    // Listen for barge-in interruption
    if (unlistenRef.current) {
      unlistenRef.current();
    }
    const unlisten = await listen("voice:interrupted", () => {
      if (Date.now() - playStartTimeRef.current < BARGE_IN_GRACE_MS) {
        console.log("[TTS] barge-in ignored during grace period");
        return;
      }
      cancelledRef.current = true;
      if (audioRef.current) {
        audioRef.current.pause();
        URL.revokeObjectURL(audioRef.current.src);
        audioRef.current = null;
      }
      playingRef.current = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    });
    unlistenRef.current = unlisten;

    invoke("voice_start_barge_in_monitor").catch((e) => {
      console.warn("[TTS] Failed to start barge-in monitor:", e);
    });

    return new Promise<void>((resolve) => {
      audio.onended = () => {
        playingRef.current = false;
        URL.revokeObjectURL(url);
        invoke("voice_stop_barge_in_monitor").catch(() => {});
        if (unlistenRef.current) {
          unlistenRef.current();
          unlistenRef.current = null;
        }
        resolve();
      };
      audio.onerror = () => {
        playingRef.current = false;
        URL.revokeObjectURL(url);
        invoke("voice_stop_barge_in_monitor").catch(() => {});
        if (unlistenRef.current) {
          unlistenRef.current();
          unlistenRef.current = null;
        }
        resolve();
      };
      audio.play().catch(() => resolve());
    });
  }, []);

  /** Play a single TTS segment (short text, no splitting). */
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
      invoke("voice_stop_barge_in_monitor").catch(() => {});
    }
  }, [stop, playBase64]);

  /** Play long text by splitting into segments and playing sequentially. */
  const speakSegments = useCallback(async (text: string, speakerId?: number) => {
    stop();
    cancelledRef.current = false;

    const segments = splitIntoSegments(text);
    console.log(`[TTS] speakSegments: ${text.length} chars → ${segments.length} segments`);
    segments.forEach((seg, i) => console.log(`[TTS]   seg[${i}]: ${seg.length} chars, "${seg.slice(0, 60)}"`));
    const voice = speakerId !== undefined ? String(speakerId) : undefined;

    for (let i = 0; i < segments.length; i++) {
      if (cancelledRef.current) {
        console.log(`[TTS] cancelled at segment ${i}/${segments.length}`);
        break;
      }
      const segment = segments[i];
      try {
        console.log(`[TTS] invoking tts_speak for segment ${i}: ${segment.length} chars`);
        const base64: string = await invoke("tts_speak", { text: segment, voice });
        console.log(`[TTS] tts_speak returned base64 length=${base64.length}`);
        if (cancelledRef.current) break;
        await playBase64(base64);
        console.log(`[TTS] segment ${i} playback completed`);
      } catch (e) {
        console.error("TTS segment failed:", e);
        break;
      }
    }

    playingRef.current = false;
    invoke("voice_stop_barge_in_monitor").catch(() => {});
  }, [stop, playBase64]);

  const isSpeaking = () => playingRef.current;

  return { speak, speakSegments, stop, isSpeaking };
}
