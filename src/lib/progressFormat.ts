/// Formatting for the Progress page. Pure functions, no React, no invoke — the only
/// units in this feature a test framework could cover if one is ever added.

/// A share, exactly as the backend rounded it.
///
/// Never re-rounds: the backend already floors a share just below whole while any word
/// remains unknown, and rounding again here could put "100%" back on a text that still has
/// something left in it.
export function formatPercent(value: number): string {
  return value.toFixed(1);
}

/// A signed change in percentage points.
export function formatDelta(points: number): string {
  const rounded = Math.round(points * 10) / 10;
  if (rounded > 0) {
    return `+${rounded.toFixed(1)}`;
  }
  return rounded.toFixed(1);
}

/// A whole number with thousands separators, in the user's own locale.
export function formatCount(value: number): string {
  return value.toLocaleString();
}

/// A date the way the page says it: "8 Sept", or with the year once it is not this one.
///
/// A reading from last year saying "8 Sept" reads as a week ago, which is the one way a
/// date can be worse than no date at all.
export function formatDay(ms: number, now: Date): string {
  const when = new Date(ms);
  const sameYear = when.getFullYear() === now.getFullYear();
  return when.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

/// How many whole documents a comparison covered, said in words.
///
/// The count is not decoration. A change drawn from three transcripts and one drawn from
/// three hundred deserve different amounts of trust, and the only way a reader can tell is
/// if the page says which it is.
export function formatCompared(
  compared: number,
  added: number,
  changed: number,
): string {
  const parts = [`${compared} compared`];
  if (added > 0) {
    parts.push(`${added} new`);
  }
  if (changed > 0) {
    parts.push(`${changed} re-transcribed`);
  }
  return parts.join(" · ");
}
