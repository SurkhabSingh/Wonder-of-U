
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

const MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// Day keys are already local days from the backend, so arithmetic runs in UTC to keep the
// machine's own zone out of it.
export function shiftDay(day: string, byDays: number): string {
  const at = new Date(`${day}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() + byDays);
  return at.toISOString().slice(0, 10);
}

/** 0 is Monday. */
export function weekdayOf(day: string): number {
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

export function monthOf(day: string): number {
  return Number(day.slice(5, 7)) - 1;
}

export function monthName(month: number): string {
  return MONTH_NAMES[month];
}

export function shortDate(day: string): string {
  return `${MONTH_NAMES[monthOf(day)]} ${Number(day.slice(8, 10))}`;
}

/** A clock time, with the date in front once it is no longer today. */
export function formatMoment(ms: number, now: Date): string {
  const when = new Date(ms);
  const time = when.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return when.toDateString() === now.toDateString() ? time : `${formatDay(ms, now)}, ${time}`;
}
