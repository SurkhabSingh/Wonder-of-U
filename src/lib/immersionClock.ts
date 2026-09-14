import { invoke } from "@tauri-apps/api/core";

const HEARTBEAT_MS = 5000;

// Module scope, not a hook ref: one clock per app, so switching pages mid-track does not
// reset the throttle and turn one stretch into two.
let lastPlaying: boolean | null = null;
let lastSource: string | null = null;
let lastSentAtMs = 0;

/**
 * Reports one reading of the audio element. Rust decides what any of it is worth.
 *
 * A change of state or of source always goes out; repeats of one state are throttled.
 * `source` distinguishes one clip or track from the next, so the position jump between
 * them starts a fresh stretch instead of reading as playback. `force` is for a move that
 * changes neither: a loop rewinding the same clip.
 */
export function reportListening(
  playing: boolean,
  positionMs: number,
  rate: number,
  source: string | null,
  force = false,
): void {
  // Called as the first statement of media event handlers: a throw here would abort the
  // handler, leaving the clip logic and its state reset unrun.
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
    /* Measurement never costs playback. */
  }
}

function wholeMs(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

function usableRate(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}
