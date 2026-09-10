import type { ActivityDay } from "../../types";

const WEEKS = 26;
const DAYS_IN_WEEK = 7;

/// Shades a day by how much arrived on it. Four steps, because more than that is a legend
/// nobody reads and fewer cannot tell a busy day from a quiet one.
function level(items: number): number {
  if (items === 0) return 0;
  if (items <= 2) return 1;
  if (items <= 5) return 2;
  return 3;
}

/// Steps back one day from a `YYYY-MM-DD` label.
///
/// The labels are dates, not instants: the backend already applied the 04:00 rollover when
/// it made them. Walking them with UTC arithmetic keeps them dates — using local time here
/// would re-introduce a timezone to strings that no longer have one, and shift the grid by
/// a day for anyone west of UTC.
function stepBack(day: string, byDays: number): string {
  const at = new Date(`${day}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() - byDays);
  return at.toISOString().slice(0, 10);
}

function weekdayOf(day: string): number {
  // 0 = Monday, so the grid reads the way a week does.
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

/// When the library was worked on, as a year-ish grid.
///
/// Drawn from the arrival time every recording already carries, so it needs nothing stored
/// and reaches back as far as the library does rather than as far as this feature does.
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
        aria-label={`${shown} active days in the last ${WEEKS} weeks`}
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
          {shown} day{shown === 1 ? "" : "s"} in the last six months
        </span>
      </div>
    </div>
  );
}
