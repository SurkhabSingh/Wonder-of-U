import type { RecordingSegment } from "../types";

// Segment editing shared by every surface that mines a timed line: the transcript
// viewer, and the subtitle list of a watch session. These were private to the
// transcript viewer until the watch feature needed exactly the same behaviour —
// copying them would have been two implementations of "what counts as the same
// sentence", which is the thing the mined/not-mined marker depends on.
//
// Everything here is a pure function over `{text, startMs, endMs}`. A subtitle cue
// from the parser has that exact shape, so nothing needs adapting.

const SENTENCE_ENDINGS = new Set([
  "。",
  "！",
  "？",
  "．",
  ".",
  "!",
  "?",
  "…",
]);

// A stable, content-derived key for a segment so an already-mined row keeps its
// "✓ Mined" marker across re-renders. Merging/splitting produces a new sentence
// (new text/timing), so its key differs and the marker naturally resets.
export function segmentMineKey(segment: RecordingSegment): string {
  return `${segment.startMs}:${segment.endMs}:${segment.text}`;
}

// Merge row i with row i+1 into one sentence spanning both time ranges. The
// joiner is script-aware: CJK scripts run without inter-word spaces, so a space
// would leave an unnatural gap in the merged sentence (and in a mined card).
export function mergeSegmentAt(
  segments: RecordingSegment[],
  index: number,
  joiner: string,
): RecordingSegment[] {
  if (index < 0 || index >= segments.length - 1) {
    return segments;
  }
  const a = segments[index];
  const b = segments[index + 1];
  const merged: RecordingSegment = {
    text: `${a.text}${joiner}${b.text}`,
    startMs: a.startMs,
    endMs: b.endMs,
  };
  return [...segments.slice(0, index), merged, ...segments.slice(index + 2)];
}

// Split row i at the first sentence-ending punctuation at or after the text
// midpoint, else at the character midpoint. Time is divided proportionally by
// the character cut index so each half keeps a plausible span.
export function splitSegmentAt(
  segments: RecordingSegment[],
  index: number,
): RecordingSegment[] {
  const segment = segments[index];
  if (!segment) {
    return segments;
  }
  const text = segment.text;
  if (text.length < 2) {
    return segments;
  }

  const midpoint = Math.floor(text.length / 2);
  let cutIndex = midpoint;
  for (let position = midpoint; position < text.length; position += 1) {
    if (SENTENCE_ENDINGS.has(text[position])) {
      // Keep the punctuation with the first sentence.
      cutIndex = position + 1;
      break;
    }
  }
  // A punctuation mark sitting at the very end leaves nothing for the second
  // half; fall back to the character midpoint in that case.
  if (cutIndex <= 0 || cutIndex >= text.length) {
    cutIndex = midpoint;
  }

  const firstText = text.slice(0, cutIndex).trim();
  const secondText = text.slice(cutIndex).trim();
  if (firstText.length === 0 || secondText.length === 0) {
    return segments;
  }

  const span = segment.endMs - segment.startMs;
  const splitMs = Math.round(segment.startMs + span * (cutIndex / text.length));
  const first: RecordingSegment = {
    text: firstText,
    startMs: segment.startMs,
    endMs: splitMs,
  };
  const second: RecordingSegment = {
    text: secondText,
    startMs: splitMs,
    endMs: segment.endMs,
  };
  return [...segments.slice(0, index), first, second, ...segments.slice(index + 1)];
}

// Index of the segment covering `positionMs`, or -1.
//
// This exists because the reading pane originally identified the playing row by
// exact start/end equality — safe only because the audio player handed back the
// very segment object the row was built from. A player's clock gives a POSITION,
// not an identity, and after a merge or split no row's bounds match a cue any
// more. Searching by containment is what keeps the highlight correct once rows
// can be edited.
export function segmentIndexAt(
  segments: RecordingSegment[],
  positionMs: number,
): number {
  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    if (positionMs >= segment.startMs && positionMs < segment.endMs) {
      return index;
    }
  }
  return -1;
}
