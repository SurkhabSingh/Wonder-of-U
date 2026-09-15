import type { CalendarDay, CalendarSpan } from "../../types";
import { formatDuration } from "../../lib/progressFormat";

const DAYS_IN_WEEK = 7;
const MAX_WEEKS = 53;
// Cells are fluid but capped, so a grid covering five days does not balloon into squares
// the size of the tiles above it.
const MAX_CELL_PX = 16;

function level(ms: number): number {
  const minutes = ms / 60000;
  if (minutes < 1) return 0;
  if (minutes < 15) return 1;
  if (minutes < 30) return 2;
  if (minutes < 60) return 3;
  return 4;
}

function shift(day: string, byDays: number): string {
  const at = new Date(`${day}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() + byDays);
  return at.toISOString().slice(0, 10);
}

function weekdayOf(day: string): number {
  // 0 = Monday, so the grid reads the way a week does.
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

function daysBetween(from: string, to: string): number {
  const span = Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`);
  return Math.round(span / 86400000);
}

// A day with no number gets no colour step. `entry.combinedMs ?? 0` would put a measured
// zero over a day nobody was counting, which is the one thing this grid must not say.
function classOf(entry: CalendarDay | undefined, within: boolean): string {
  if (!within) {
    return "progress-cal-cell is-outside";
  }
  if (!entry || entry.combinedMs === null) {
    return "progress-cal-cell is-uncounted";
  }
  return `progress-cal-cell l${level(entry.combinedMs)}`;
}

function titleOf(day: string, entry: CalendarDay | undefined, within: boolean): string {
  if (!within) {
    return "";
  }
  if (!entry || entry.combinedMs === null) {
    return `${day} — before time was being counted`;
  }
  const said = [
    day,
    entry.combinedMs === 0 ? "nothing counted" : formatDuration(entry.combinedMs),
  ];
  if (entry.madeCard) {
    said.push("a card was made");
  }
  if (entry.addedMaterial) {
    said.push("material added");
  }
  return said.join(" · ");
}

export function StudyCalendar({
  span,
  today,
}: {
  span: CalendarSpan;
  today: string;
}) {
  const firstDay = span.firstDay;
  if (firstDay === null) {
    return null;
  }

  const byDay = new Map(span.days.map((entry) => [entry.day, entry]));
  const lastColumn = shift(today, -weekdayOf(today));
  const firstColumn = shift(firstDay, -weekdayOf(firstDay));
  const weeks = Math.min(
    MAX_WEEKS,
    Math.max(1, Math.round(daysBetween(firstColumn, lastColumn) / DAYS_IN_WEEK) + 1),
  );
  const gridStart = shift(lastColumn, -(weeks - 1) * DAYS_IN_WEEK);

  const cells: { day: string; entry: CalendarDay | undefined; within: boolean }[] = [];
  for (let row = 0; row < DAYS_IN_WEEK; row += 1) {
    for (let week = 0; week < weeks; week += 1) {
      const day = shift(gridStart, week * DAYS_IN_WEEK + row);
      cells.push({
        day,
        entry: byDay.get(day),
        within: day >= firstDay && day <= today,
      });
    }
  }

  const counted = span.days.filter(
    (entry) => entry.combinedMs !== null && entry.combinedMs > 0,
  ).length;

  return (
    <div className="progress-cal">
      <div
        className="progress-cal-grid"
        style={{
          gridTemplateColumns: `repeat(${weeks}, minmax(0, 1fr))`,
          maxWidth: `${weeks * MAX_CELL_PX}px`,
        }}
        role="img"
        aria-label={`${counted} days with time counted since ${span.countedFrom}`}
      >
        {cells.map((cell) => (
          <i
            key={cell.day}
            className={[
              classOf(cell.entry, cell.within),
              cell.entry?.addedMaterial ? "has-material" : "",
              cell.day === span.countedFrom ? "is-seam" : "",
              cell.day === today ? "is-today" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            title={titleOf(cell.day, cell.entry, cell.within)}
          >
            {cell.entry?.madeCard ? (
              <span className="progress-cal-card-dot" aria-hidden="true" />
            ) : null}
          </i>
        ))}
      </div>

      <div className="progress-cal-legend">
        <span>less</span>
        <i className="progress-cal-cell l0" />
        <i className="progress-cal-cell l1" />
        <i className="progress-cal-cell l2" />
        <i className="progress-cal-cell l3" />
        <i className="progress-cal-cell l4" />
        <span>more</span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-uncounted" /> not counted yet
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell l0">
            <span className="progress-cal-card-dot" aria-hidden="true" />
          </i>{" "}
          a card was made
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell l0 has-material" /> material added
        </span>
      </div>

      <p className="progress-cal-since">
        Counting time since {span.countedFrom}
        {span.droppedEvidence > 0
          ? ` · ${span.droppedEvidence} item${
              span.droppedEvidence === 1 ? "" : "s"
            } carried no usable date`
          : ""}
      </p>
    </div>
  );
}
