/// Formatting for the Progress page. Pure, so a test framework could cover it.

/// Exactly as the backend rounded it. Re-rounding could put "100%" back on a text that
/// still has something left in it.
export function formatPercent(value: number): string {
  return value.toFixed(1);
}

export function formatDelta(points: number): string {
  const rounded = Math.round(points * 10) / 10;
  if (rounded > 0) {
    return `+${rounded.toFixed(1)}`;
  }
  return rounded.toFixed(1);
}

/// Rounded down, so a total never claims time the library does not hold. Under a minute
/// says so rather than reading as nothing.
export function formatDuration(ms: number): string {
  const minutes = Math.floor(ms / 60000);
  if (minutes === 0) {
    return ms > 0 ? "under a minute" : "0m";
  }
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) {
    return `${rest}m`;
  }
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

export function formatCount(value: number): string {
  return value.toLocaleString();
}

/// "8 Sept", with the year once it is not this one: a reading from last year saying
/// "8 Sept" reads as a week ago.
export function formatDay(ms: number, now: Date): string {
  const when = new Date(ms);
  const sameYear = when.getFullYear() === now.getFullYear();
  return when.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

/// How many documents a comparison covered. A change from three transcripts and one from
/// three hundred deserve different trust, and only this says which it is.
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
