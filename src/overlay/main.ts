import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Matches the payload emitted by `signal_recording_indicator`.
type IndicatorState = "recording" | "saved" | "failed";

interface IndicatorPayload {
  state: IndicatorState;
  label: string;
}

const VISIBLE_DURATION_MS = 3200;

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

card.addEventListener("transitionend", (event) => {
  if (event.propertyName === "opacity" && !card.classList.contains("visible")) {
    hideWindow();
  }
});

// Paints the input-level bar from a peak reading (0..1).
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
  card.classList.toggle("state-recording", payload.state === "recording");
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
