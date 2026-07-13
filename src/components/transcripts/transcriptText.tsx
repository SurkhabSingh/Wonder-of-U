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

export function countMatches(text: string, query: string): number {
  const trimmed = query.trim();
  if (!trimmed) {
    return 0;
  }

  const pattern = new RegExp(escapeRegExp(trimmed), "gi");
  return (text.match(pattern) ?? []).length;
}

export function highlightMatches(text: string, query: string): ReactNode {
  const trimmed = query.trim();
  if (!trimmed) {
    return text;
  }

  const pattern = new RegExp(`(${escapeRegExp(trimmed)})`, "gi");
  const parts = text.split(pattern);

  return parts.map((part, index) =>
    // String.split with a capturing group places the matched text at odd
    // indices; everything else is untouched surrounding text.
    index % 2 === 1 ? (
      <mark key={index} className="transcript-mark">
        {part}
      </mark>
    ) : (
      part
    ),
  );
}
