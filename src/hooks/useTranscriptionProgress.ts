import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

// The backend streams whisper's `progress = N%` (0..100) on this event while a
// transcription runs — one file at a time, so the value resets 0→100 per file in a
// batch. Mirrors the `recording-level` / `youtube-progress` event hooks.
const TRANSCRIPTION_PROGRESS_EVENT = "transcription-progress";

/**
 * Subscribes to live transcription progress. Returns 0..100 while `active` (the
 * transcribing phase), starting at 0 so the bar shows immediately, or `null` when
 * idle so the overlay hides the bar — without waiting on a trailing event.
 */
export function useTranscriptionProgress(active: boolean): number | null {
  const [progress, setProgress] = useState<number | null>(null);

  useEffect(() => {
    const unlisten = listen<number>(
      TRANSCRIPTION_PROGRESS_EVENT,
      ({ payload }) => {
        if (typeof payload === "number") {
          setProgress(Math.max(0, Math.min(100, payload)));
        }
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // Reset to 0 when a transcription starts (bar visible from the outset) and clear
  // it the instant the phase ends.
  useEffect(() => {
    setProgress(active ? 0 : null);
  }, [active]);

  return active ? progress : null;
}
