// The recording indicator overlay: a small ShadowPlay-style toast that slides
// into a screen corner when recording starts, stops, or fails. It lives in its
// own transparent, click-through window (see `recording_indicator.rs`) and must
// stay out of React — nothing here needs a framework, and the lighter the bundle
// the faster the toast can paint over whatever the user is watching.

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Matches the payload emitted by `signal_recording_indicator`.
type IndicatorState = "recording" | "saved" | "failed";

interface IndicatorPayload {
  state: IndicatorState;
  label: string;
}

// How long the toast stays fully visible before it fades out. Long enough not to
// be missed while glancing away from the game/video that prompted it.
const VISIBLE_DURATION_MS = 3200;

// Per-state accent (the icon disc + card rail) and the glyph inside the disc. The
// recording state is a plain red disc — the record symbol itself — so it carries
// no glyph; the terminal states get a check / bang.
const ACCENTS: Record<IndicatorState, string> = {
  recording: "#ff4d4f",
  saved: "#34c759",
  failed: "#ffb020",
};
const GLYPHS: Record<IndicatorState, string> = {
  recording: "",
  saved: "✓",
  failed: "!",
};

// Lifts quiet input into a visible bar without pinning loud peaks to full —
// matches the Home meter (`RecordingLevelMeter.tsx`) so both read identically.
const LEVEL_GAMMA = 0.6;

const card = document.getElementById("card") as HTMLDivElement;
const icon = document.getElementById("icon") as HTMLSpanElement;
const title = document.getElementById("title") as HTMLSpanElement;
const levelFill = document.getElementById("overlay-level-fill") as HTMLSpanElement;

// A single pending fade-out timer, reset on every event so rapid start/stop
// bursts never hide the toast while it is still announcing the latest state.
let hideTimer: number | undefined;

function hideWindow(): void {
  void getCurrentWindow().hide();
}

// Once the fade-out transition finishes, drop the window so it stops compositing
// over the content beneath it. Guarded on `opacity` because the same transition
// fires at the end of the slide-in too.
card.addEventListener("transitionend", (event) => {
  if (event.propertyName === "opacity" && !card.classList.contains("visible")) {
    hideWindow();
  }
});

// Paints the input-level bar from a peak reading (0..1). Only touches the DOM
// while the recording toast is actually on screen — the backend keeps streaming
// readings after the toast has auto-hidden (capture is still running), and there
// is no point repainting a hidden, non-recording card.
function setLevel(level: number): void {
  if (
    !card.classList.contains("visible") ||
    !card.classList.contains("state-recording")
  ) {
    return;
  }
  const clamped = level > 0 ? level : 0;
  const pct = Math.min(100, Math.round(Math.pow(clamped, LEVEL_GAMMA) * 100));
  levelFill.style.clipPath = `inset(0 ${100 - pct}% 0 0)`;
}

function showSignal(payload: IndicatorPayload): void {
  card.style.setProperty("--accent", ACCENTS[payload.state]);
  icon.textContent = GLYPHS[payload.state];
  title.textContent = payload.label;
  // Only a live recording pulses; the class also lets the CSS scope the pulse
  // and the level bar to the recording state.
  card.classList.toggle("state-recording", payload.state === "recording");
  // Start each recording toast with an empty bar so it fills from the live
  // readings rather than flashing the previous session's last peak.
  if (payload.state === "recording") {
    levelFill.style.clipPath = "inset(0 100% 0 0)";
  }

  // Force a reflow so re-triggering while already visible still animates in.
  void card.offsetWidth;
  card.classList.add("visible");

  if (hideTimer !== undefined) {
    window.clearTimeout(hideTimer);
  }
  hideTimer = window.setTimeout(() => {
    card.classList.remove("visible");
  }, VISIBLE_DURATION_MS);
}

void listen<IndicatorPayload>("recording-indicator", (event) => {
  showSignal(event.payload);
});

void listen<number>("recording-level", (event) => {
  setLevel(event.payload);
});
