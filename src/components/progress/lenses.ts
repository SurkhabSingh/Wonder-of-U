import type { CalendarDay, CalendarSpan, CardCounts, MaterialCounts } from "../../types";
import { formatCount, formatDuration, shortDate } from "../../lib/progressFormat";

export type Lens = "time" | "cards" | "material";

// What a day with no number says: `tip` on the day itself, `band` across a run of them.
export type Gap = { tip: string; band: string };

export type LensSpec = {
  label: string;
  title: string;
  valueOf: (entry: CalendarDay) => number | null;
  format: (value: number) => string;
  total: (value: number) => string;
  // Upper bounds of the first three fill steps, printed in the legend.
  steps: number[];
  stepLabels: string[];
  ticks: number[];
  formatTick: (value: number) => string;
  detailHeading: string | null;
  detailOf: (entry: CalendarDay) => string | null;
  gap: (day: string, span: CalendarSpan) => Gap;
};

const MINUTE = 60000;
const COUNT_TICKS = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000];

function counted(count: number, one: string, many: string): string {
  return `${formatCount(count)} ${count === 1 ? one : many}`;
}

function listed(parts: (string | null)[]): string | null {
  const kept = parts.filter((part): part is string => part !== null);
  return kept.length > 0 ? kept.join(" · ") : null;
}

function cardTotal(counts: CardCounts): number {
  return counts.word + counts.line + counts.transcript + counts.unsorted;
}

function cardKinds(counts: CardCounts): string | null {
  return listed([
    counts.word > 0 ? counted(counts.word, "word", "words") : null,
    counts.line > 0 ? counted(counts.line, "sentence", "sentences") : null,
    counts.transcript > 0 ? counted(counts.transcript, "transcript", "transcripts") : null,
    counts.unsorted > 0 ? `${formatCount(counts.unsorted)} unsorted` : null,
  ]);
}

function materialKinds(counts: MaterialCounts): string | null {
  return listed([
    counts.recordings > 0 ? `${formatCount(counts.recordings)} audio` : null,
    counts.videos > 0 ? counted(counts.videos, "video", "videos") : null,
  ]);
}

const TIME: LensSpec = {
  label: "Time",
  title: "Time played",
  valueOf: (entry) => entry.combinedMs,
  format: formatDuration,
  total: formatDuration,
  steps: [15 * MINUTE, 30 * MINUTE, 60 * MINUTE],
  stepLabels: ["<15m", "15m", "30m", "1h+"],
  ticks: [1, 2, 5, 10, 15, 30, 60, 120, 240, 480].map((minutes) => minutes * MINUTE),
  formatTick: formatDuration,
  detailHeading: null,
  detailOf: () => null,
  gap: (_day, span) =>
    span.countedFrom === null
      ? { tip: "Could not be read", band: "Could not be read" }
      : { tip: "Not counted", band: "Not counted yet" },
};

const CARDS: LensSpec = {
  label: "Cards",
  title: "Cards made",
  valueOf: (entry) => (entry.cards === null ? null : cardTotal(entry.cards)),
  format: (value) => counted(value, "card", "cards"),
  total: formatCount,
  steps: [3, 6, 10],
  stepLabels: ["1–2", "3–5", "6–9", "10+"],
  ticks: COUNT_TICKS,
  formatTick: formatCount,
  detailHeading: "Kind",
  detailOf: (entry) => (entry.cards === null ? null : cardKinds(entry.cards)),
  gap: (day, span) => {
    if (span.cards === null) {
      return { tip: "Not counted", band: "Not counted yet" };
    }
    if (day > span.cards.on) {
      return { tip: "Not counted yet", band: `Last counted ${shortDate(span.cards.on)}` };
    }
    return { tip: "No data", band: "Before your first card" };
  },
};

const MATERIAL: LensSpec = {
  label: "Material",
  title: "Material added",
  valueOf: (entry) =>
    entry.material === null ? null : entry.material.recordings + entry.material.videos,
  format: (value) => counted(value, "item", "items"),
  total: formatCount,
  steps: [2, 4, 8],
  stepLabels: ["1", "2–3", "4–7", "8+"],
  ticks: COUNT_TICKS,
  formatTick: formatCount,
  detailHeading: "Kind",
  detailOf: (entry) => (entry.material === null ? null : materialKinds(entry.material)),
  gap: () => ({ tip: "No data", band: "Before your library" }),
};

export const LENSES: Record<Lens, LensSpec> = { time: TIME, cards: CARDS, material: MATERIAL };

export const LENS_ORDER: Lens[] = ["time", "cards", "material"];

/** 1 to 4, for a value above zero. */
export function stepOf(value: number, steps: number[]): number {
  const below = steps.findIndex((limit) => value < limit);
  return below === -1 ? steps.length + 1 : below + 1;
}
