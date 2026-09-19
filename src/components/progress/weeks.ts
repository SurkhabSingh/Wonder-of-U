import { monthName, monthOf, shiftDay, weekdayOf } from "../../lib/progressFormat";

export const DAYS_IN_WEEK = 7;

/** The Mondays of the `count` weeks ending with the one holding `today`, oldest first. */
export function weeksEnding(today: string, count: number): string[] {
  const first = shiftDay(today, -weekdayOf(today) - (count - 1) * DAYS_IN_WEEK);
  return Array.from({ length: count }, (_, week) => shiftDay(first, week * DAYS_IN_WEEK));
}

// A month is labelled on the first week it owns, skipped when the next is too close to fit.
export function monthMarks(mondays: string[]): { week: number; label: string }[] {
  const marks = mondays.flatMap((monday, week) =>
    week === 0 || monthOf(shiftDay(monday, -DAYS_IN_WEEK)) !== monthOf(monday)
      ? [{ week, label: monthName(monthOf(monday)) }]
      : [],
  );
  return marks.filter(
    (mark, index) => index === marks.length - 1 || marks[index + 1].week - mark.week >= 3,
  );
}
