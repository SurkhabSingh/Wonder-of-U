import type { ActivityDay } from "../../types";

// A full year. The column count decides cell size in a stretched grid, and a year shows
// the start of the library as empty space rather than cropping it out.
const WEEKS = 53;
const DAYS_IN_WEEK = 7;

/// Four steps: more is a legend nobody reads, fewer cannot tell busy from quiet.
function level(items: number): number {
  if (items === 0) return 0;
  if (items <= 2) return 1;
  if (items <= 5) return 2;
  return 3;
}

/// Labels are dates, not instants. UTC arithmetic keeps them dates; local time would shift
/// the grid by a day for anyone west of UTC.
function stepBack(day: string, byDays: number): string {
  const at = new Date(`${day}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() - byDays);
  return at.toISOString().slice(0, 10);
}

function weekdayOf(day: string): number {
  // 0 = Monday, so the grid reads the way a week does.
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

/// Drawn from arrival times recordings already carry, so it needs nothing stored and
/// reaches back as far as the library rather than as far as this feature.
export function ActivityCalendar({
  days,
  today,
}: {
  days: ActivityDay[];
  today: string;
}) {
  const counts = new Map(days.map((entry) => [entry.day, entry.items]));

  // The grid ends on the week holding today, so the last column is the current week.
  const lastColumnStart = stepBack(today, weekdayOf(today));
  const cells: { day: string; items: number }[] = [];
  for (let row = 0; row < DAYS_IN_WEEK; row += 1) {
    for (let week = WEEKS - 1; week >= 0; week -= 1) {
      const day = stepBack(lastColumnStart, week * DAYS_IN_WEEK - row);
      cells.push({ day, items: counts.get(day) ?? 0 });
    }
  }

  const shown = cells.filter((cell) => cell.items > 0).length;

  return (
    <div className="activity">
      <div
        className="activity-grid"
        style={{ gridTemplateColumns: `repeat(${WEEKS}, 1fr)` }}
        role="img"
        aria-label={`${shown} active days in the last year`}
      >
        {cells.map((cell) => (
          <i
            key={cell.day}
            className={`activity-cell l${level(cell.items)}`}
            title={
              cell.items === 0
                ? cell.day
                : `${cell.day}: ${cell.items} item${cell.items === 1 ? "" : "s"}`
            }
          />
        ))}
      </div>
      <div className="activity-legend">
        <span>less</span>
        <i className="activity-cell l0" />
        <i className="activity-cell l1" />
        <i className="activity-cell l2" />
        <i className="activity-cell l3" />
        <span>more</span>
        <span className="activity-count">
          {shown} day{shown === 1 ? "" : "s"} in the last year
        </span>
      </div>
    </div>
  );
}
