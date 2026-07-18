import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

// The backend's capture loop emits a single peak amplitude (0.0..=1.0) on a
// steady ~75ms cadence while recording, and one final 0 when it stops. This hook
// surfaces the latest reading for the input meter.
const RECORDING_LEVEL_EVENT = "recording-level";

/**
 * Subscribes to the live recording input level. Returns the latest peak
 * amplitude in the range 0..1, or 0 whenever `active` is false — so the meter
 * rests the instant recording is no longer the current phase, without waiting on
 * a trailing event that a hidden window might miss.
 */
export function useRecordingLevel(active: boolean): number {
  const [level, setLevel] = useState(0);

  useEffect(() => {
    const unlisten = listen<number>(RECORDING_LEVEL_EVENT, ({ payload }) => {
      setLevel(typeof payload === "number" && payload >= 0 ? payload : 0);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // A stale peak must not linger on screen once capture ends.
  useEffect(() => {
    if (!active) {
      setLevel(0);
    }
  }, [active]);

  return active ? level : 0;
}
