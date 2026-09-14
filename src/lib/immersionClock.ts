import { invoke } from "@tauri-apps/api/core";

const HEARTBEAT_MS = 5000;

let lastPlaying: boolean | null = null;
let lastSource: string | null = null;
let lastSentAtMs = 0;

export function reportListening(
  playing: boolean,
  positionMs: number,
  rate: number,
  source: string | null,
  force = false,
): void {
  // First statement of media event handlers: a throw would abort the rest of one.
  try {
    const now = Date.now();
    if (
      !force &&
      playing === lastPlaying &&
      source === lastSource &&
      now - lastSentAtMs < HEARTBEAT_MS
    ) {
      return;
    }
    lastPlaying = playing;
    lastSource = source;
    lastSentAtMs = now;
    void invoke("record_listening_sample", {
      playing,
      positionMs: wholeMs(positionMs),
      rate: usableRate(rate),
    }).catch((caught) => {
      if (import.meta.env.DEV) {
        console.debug("record_listening_sample failed:", caught);
      }
    });
  } catch {
  }
}

function wholeMs(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

function usableRate(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}
