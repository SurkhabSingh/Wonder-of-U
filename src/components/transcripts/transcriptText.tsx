import type { ReactNode } from "react";

// Rows split on the transcript's own newlines only. Whole-document
// translations often arrive as a single blob, which becomes one long row.
export function splitTranscriptSegments(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Reduces a sentence to the form the backend compares against, mirroring the tail of
// the Rust `normalize_mined_text`: collapse whitespace runs and trim. Transcript
// segments are plain text, so the HTML/ruby half of that normalizer has no counterpart
// here — `load_mined_sentences` hands back sentences already stripped.
export function normalizeSegmentText(text: string): string {
  return text.split(/\s+/).filter(Boolean).join(" ");
}

export function countMatches(text: string, query: string): number {
  const trimmed = query.trim();
  if (!trimmed) {
    return 0;
  }

  const pattern = new RegExp(escapeRegExp(trimmed), "gi");
  return (text.match(pattern) ?? []).length;
}

export function highlightMatches(
  text: string,
  query: string,
  // Which occurrence within THIS line is the one being stepped to, or null when
  // the active match is on another line. Counted per line rather than across the
  // transcript so a row can be highlighted without knowing where it sits.
  activeOccurrence: number | null = null,
): ReactNode {
  const trimmed = query.trim();
  if (!trimmed) {
    return text;
  }

  const pattern = new RegExp(`(${escapeRegExp(trimmed)})`, "gi");
  const parts = text.split(pattern);

  let occurrence = -1;
  return parts.map((part, index) => {
    // String.split with a capturing group places the matched text at odd
    // indices; everything else is untouched surrounding text.
    if (index % 2 === 0) {
      return part;
    }
    occurrence += 1;
    return (
      <mark
        key={index}
        className={`transcript-mark${
          occurrence === activeOccurrence ? " is-active" : ""
        }`}
      >
        {part}
      </mark>
    );
  });
}
