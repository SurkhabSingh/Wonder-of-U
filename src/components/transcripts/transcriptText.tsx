import type { ReactNode } from "react";

export function splitTranscriptSegments(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

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
